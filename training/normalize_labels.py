import argparse
import csv
import json
import random
import re
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np

from taxonomy import ADULT_TERMS, EXCLUDED_AGE, HERE, normalise, read_hard_negatives
from workdir import LABELS, ROOT

NORMALIZED = ROOT / "data" / "normalized"
TEXT_MODEL = "google/siglip2-base-patch16-224"
LEADING = re.compile(r"^(?:[\s*#\-\u2022>]+|\d+[.)]\s+)+")
ARTICLE = re.compile(r"^(the|a|an) ")
BUCKETS = [(1, 1), (2, 4), (5, 9), (10, 19), (20, 29), (30, 49), (50, 99), (100, 499), (500, None)]
FLOORS = [10, 20, 30, 50]


def spelling(label):
    s = LEADING.sub("", label.strip())
    s = s.replace("&", " and ")
    s = normalise(s)
    s = ARTICLE.sub("", s)
    return normalise(s) if s else ""


def compact(label):
    return re.sub(r"[\s\-'.]", "", label)


def read_files(path):
    files = defaultdict(set)
    with open(path) as f:
        for line in f:
            parts = line.rstrip("\n").split("\t")
            if len(parts) == 3 and parts[2].strip():
                files[parts[0]].add(parts[2])
    return files


def counts_of(files, mapping):
    counts = Counter()
    for labels in files.values():
        counts.update({mapping[l] for l in labels if mapping.get(l)})
    return counts


def spelling_map(raw_labels, files):
    first = {r: spelling(r) for r in raw_labels}
    counts = counts_of(files, first)
    by_key = defaultdict(list)
    for s in counts:
        by_key[compact(s)].append(s)
    canonical = {}
    for forms in by_key.values():
        best = max(forms, key=lambda s: (counts[s], "-" in s, -len(s), s))
        for s in forms:
            canonical[s] = best
    return {r: canonical.get(s, "") for r, s in first.items()}


def kind(label):
    age, adult = bool(EXCLUDED_AGE.search(label)), bool(ADULT_TERMS.search(label))
    return "age and adult" if age and adult else "age" if age else "adult" if adult else ""


def digits(label):
    return re.findall(r"\d+", label)


def may_merge(a, b, class_of, hard_pairs):
    ka, kb = kind(a), kind(b)
    if ka != kb or ka == "age and adult" or digits(a) != digits(b):
        return False
    ca, cb = class_of.get(a), class_of.get(b)
    if ca and ca != cb:
        return False
    return (a, b) not in hard_pairs and (b, a) not in hard_pairs


def centred(vectors):
    v = vectors - vectors.mean(0, keepdims=True)
    return v / np.maximum(np.linalg.norm(v, axis=1, keepdims=True), 1e-12)


def merge_by_meaning(labels, counts, vectors, threshold, allowed, block=1024):
    order = sorted(range(len(labels)), key=lambda i: (-counts[labels[i]], labels[i]))
    v = vectors[order].astype(np.float32)
    is_head = np.zeros(len(order), dtype=bool)
    head_of, best_sim = {}, {}
    for start in range(0, len(order), block):
        sims = v[start : start + block] @ v.T
        for r in range(sims.shape[0]):
            i = start + r
            row = np.where(is_head[:i], sims[r, :i], -np.inf)
            a = labels[order[i]]
            picked = None
            for j in np.argsort(-row):
                if row[j] < threshold:
                    break
                if allowed(a, labels[order[j]]):
                    picked = j
                    break
            top = int(np.argmax(row)) if i else None
            if top is not None and np.isfinite(row[top]):
                best_sim[a] = (labels[order[top]], float(row[top]))
            if picked is None:
                is_head[i] = True
                head_of[a] = (a, 1.0)
            else:
                head_of[a] = (labels[order[picked]], float(row[picked]))
    return head_of, best_sim


def embed_text(texts, model_name=TEXT_MODEL, batch=256):
    import torch
    from transformers import AutoModel, AutoTokenizer

    tok = AutoTokenizer.from_pretrained(model_name)
    model = AutoModel.from_pretrained(model_name).eval()
    out = []
    with torch.no_grad():
        for i in range(0, len(texts), batch):
            ids = tok(texts[i : i + batch], padding="max_length", max_length=64, truncation=True, return_tensors="pt")
            out.append(model.get_text_features(input_ids=ids["input_ids"]).float().numpy())
            print(f"embedded {min(i + batch, len(texts))}/{len(texts)}", flush=True)
    return np.concatenate(out)


def taxonomy_classes(path=HERE / "taxonomy.json"):
    taxonomy = json.loads(Path(path).read_text())
    class_of = {}
    for c in taxonomy["classes"]:
        for n in c["normalised"] + [c["name"]]:
            class_of[spelling(n)] = c["name"]
    return class_of


def bucket_table(counts):
    total = sum(counts.values())
    rows = []
    for lo, hi in BUCKETS:
        sel = [c for c in counts.values() if c >= lo and (hi is None or c <= hi)]
        rows.append({"bucket": f"{lo}" if lo == hi else f"{lo}+" if hi is None else f"{lo}-{hi}", "labels": len(sel), "uses": sum(sel), "share": sum(sel) / max(1, total)})
    return rows


def floors(counts):
    return {f: sum(1 for c in counts.values() if c >= f) for f in FLOORS}


def groups_of(mapping, files):
    raw_counts = Counter()
    for labels in files.values():
        raw_counts.update(labels)
    groups = defaultdict(Counter)
    for raw, norm in mapping.items():
        if norm:
            groups[norm][raw] += raw_counts[raw]
    return groups


def reviewed_map(spelled, class_of):
    return {r: class_of.get(s, s) for r, s in spelled.items()}


def cmd_embed(args):
    files = read_files(Path(args.labels))
    raw = sorted({l for labels in files.values() for l in labels})
    spelled = spelling_map(raw, files)
    labels = sorted({s for s in spelled.values() if s} | set(taxonomy_classes().values()))
    vectors = embed_text(labels)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    np.save(out / "text_vectors.npy", vectors)
    (out / "text_labels.json").write_text(json.dumps(labels))


def load(args):
    files = read_files(Path(args.labels))
    raw = sorted({l for labels in files.values() for l in labels})
    spelled = spelling_map(raw, files)
    reviewed = reviewed_map(spelled, taxonomy_classes())
    out = Path(args.out)
    cached = json.loads((out / "text_labels.json").read_text())
    vectors = centred(np.load(out / "text_vectors.npy"))
    index = {l: i for i, l in enumerate(cached)}
    labels = sorted({s for s in reviewed.values() if s})
    return files, raw, spelled, reviewed, labels, vectors[[index[l] for l in labels]]


def allowed_fn():
    class_of = taxonomy_classes()
    hard = {(spelling(a), spelling(b)) for a, b in read_hard_negatives()}
    return lambda a, b: may_merge(a, b, class_of, hard)


def cmd_sample(args):
    files, raw, spelled, reviewed, labels, vectors = load(args)
    counts = counts_of(files, reviewed)
    _, best = merge_by_meaning(labels, counts, vectors, 2.0, allowed_fn())
    allowed = allowed_fn()
    rng = random.Random(args.seed)
    rows = []
    edges = np.arange(args.low, 1.0001, args.step)
    for lo, hi in zip(edges[:-1], edges[1:]):
        pool = sorted((a, h, s) for a, (h, s) in best.items() if lo <= s < hi)
        for a, h, s in rng.sample(pool, min(args.per_bin, len(pool))):
            rows.append({"bin": f"{lo:.2f}", "label": a, "head": h, "similarity": f"{s:.3f}", "label_files": counts[a], "head_files": counts[h], "allowed": "yes" if allowed(a, h) else "no", "same": ""})
    write_csv(Path(args.out) / "pair_sample.csv", rows)
    print(f"{len(rows)} pairs written to {Path(args.out) / 'pair_sample.csv'}")


def write_csv(path, rows):
    with open(path, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)


def read_csv(path):
    with open(path) as f:
        return list(csv.DictReader(f))


def apply_pair_grades(rows, same):
    for i, r in enumerate(rows):
        r["same"] = "yes" if i in same else "no"
    return rows


def precision_by_bin(rows):
    table = defaultdict(Counter)
    for r in rows:
        if r["same"] in ("yes", "no") and r.get("allowed", "yes") == "yes":
            table[r["bin"]][r["same"]] += 1
    return {b: (c["yes"], c["yes"] + c["no"]) for b, c in sorted(table.items())}


def cmd_grade(args):
    path = Path(args.out) / "pair_sample.csv"
    rows = read_csv(path)
    if args.same is not None:
        rows = apply_pair_grades(rows, {int(x) for x in args.same.split()})
        write_csv(path, rows)
    for b, (yes, n) in precision_by_bin(rows).items():
        print(f"{b}\t{yes}/{n}\t{yes / n:.2f}")


def cmd_build(args):
    files, raw, spelled, reviewed, labels, vectors = load(args)
    counts = counts_of(files, reviewed)
    head_of, best = merge_by_meaning(labels, counts, vectors, args.threshold, allowed_fn())
    final = {r: head_of[s][0] if s else "" for r, s in reviewed.items()}
    out = Path(args.out)
    with open(out / "label_map.tsv", "w") as f:
        f.write("raw\tspelling\treviewed\tnormalized\tsimilarity\tkind\n")
        for r in raw:
            s = reviewed[r]
            f.write(f"{r}\t{spelled[r]}\t{s}\t{final[r]}\t{head_of[s][1]:.3f}\t{kind(s)}\n" if s else f"{r}\t\t\t\t\t\n")
    lower = {r: r.strip().lower() for r in raw}
    steps = {"raw": {r: r for r in raw}, "lowercase": lower, "spelling": spelled, "reviewed merges": reviewed, "meaning": final}
    summary = {"files": len(files), "threshold": args.threshold, "steps": {}}
    for name, mapping in steps.items():
        c = counts_of(files, mapping)
        summary["steps"][name] = {
            "distinct": len(c),
            "uses": sum(c.values()),
            "mean_files_per_label": sum(c.values()) / max(1, len(c)),
            "floors": floors(c),
            "buckets": bucket_table(c),
        }
    groups = groups_of(final, files)
    spelled_groups = groups_of(reviewed, files)
    meaning_merges = sorted((s, h, sim) for s, (h, sim) in head_of.items() if h != s)
    rng = random.Random(args.seed)
    final_counts = counts_of(files, final)
    summary["random_merges"] = [{"label": s, "into": h, "similarity": round(sim, 3), "label_files": counts[s]} for s, h, sim in rng.sample(meaning_merges, min(30, len(meaning_merges)))]
    biggest = sorted(groups, key=lambda g: (-(len(groups[g]) - 1), -final_counts[g]))[: args.top]
    summary["biggest_groups"] = [
        {"label": g, "files": final_counts[g], "raw_variants": len(groups[g]), "spelling_forms": sorted({reviewed[r] for r in groups[g]}), "raw": [r for r, _ in groups[g].most_common()]}
        for g in biggest
    ]
    summary["unsure"] = [{"label": s, "into": h, "similarity": round(sim, 3), "label_files": counts[s]} for s, h, sim in meaning_merges if sim < args.threshold + args.unsure_band]
    summary["spelling_merges"] = sum(1 for g in spelled_groups.values() if len(g) > 1)
    summary["meaning_merged_labels"] = len(meaning_merges)
    summary["age_and_adult_labels_kept_apart"] = sorted(s for s in labels if kind(s) == "age and adult")
    (out / "summary.json").write_text(json.dumps(summary, indent=1, ensure_ascii=False))
    print(json.dumps({k: summary["steps"][k]["distinct"] for k in steps} | {"meaning_merged": len(meaning_merges)}))


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    e = sub.add_parser("embed")
    s = sub.add_parser("sample")
    s.add_argument("--per-bin", type=int, default=20)
    s.add_argument("--low", type=float, default=0.6)
    s.add_argument("--step", type=float, default=0.05)
    s.add_argument("--seed", type=int, default=1539)
    b = sub.add_parser("build")
    b.add_argument("--threshold", type=float, required=True)
    b.add_argument("--unsure-band", type=float, default=0.02)
    b.add_argument("--top", type=int, default=30)
    b.add_argument("--seed", type=int, default=1539)
    g = sub.add_parser("grade")
    g.add_argument("same", nargs="?")
    for a in (e, s, b, g):
        a.add_argument("--labels", default=str(LABELS / "labels.tsv"))
        a.add_argument("--out", default=str(NORMALIZED))
    args = p.parse_args()
    {"embed": cmd_embed, "sample": cmd_sample, "build": cmd_build, "grade": cmd_grade}[args.cmd](args)


if __name__ == "__main__":
    main()
