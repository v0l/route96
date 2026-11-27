import argparse
import csv
import json
import random
from collections import Counter, defaultdict
from pathlib import Path

from export_dataset import fetch_batch, kubectl
from taxonomy import class_lookup, labels_to_classes
from workdir import DATA, ROOT

HERE = Path(__file__).parent
HELDOUT = ROOT / "data" / "heldout"
QUERY = (
    "select lower(hex(u.id)), u.mime_type, group_concat(l.label separator '|') "
    "from uploads u join upload_labels l on l.file = u.id and l.model = '{model}' "
    "where u.banned = 0 and u.mime_type in ('image/jpeg', 'image/png', 'image/webp') group by u.id"
)
SAMPLE_FIELDS = ["index", "sha256", "class", "side", "probability", "verdict"]
VERDICTS = ("model", "qwen", "unclear")


def parse_labelled(raw):
    out = {}
    for line in raw.splitlines():
        parts = line.split("\t")
        if len(parts) == 3:
            out[parts[0]] = parts[2].split("|")
    return out


def pick(labelled, lookup, exclude, count, seed):
    pool = sorted(s for s, labels in labelled.items() if s not in exclude and labels_to_classes(labels, lookup))
    random.Random(seed).shuffle(pool)
    return pool[:count]


def parse_predictions(path):
    preds = {}
    with open(path) as f:
        for r in csv.DictReader(f, delimiter="\t"):
            if r["labels"].startswith("error: "):
                continue
            preds[r["file"]] = {c: float(p) for c, p in (x.rsplit("=", 1) for x in r["labels"].split("|") if x)}
    return preds


def agreement(truth, preds, classes):
    table = {c: Counter() for c in classes}
    for sha, probs in preds.items():
        qwen = truth[sha]
        for c in classes:
            t, p = c in qwen, c in probs
            table[c]["tp" if t and p else "fn" if t else "fp" if p else "tn"] += 1
    return table


def scores(counts):
    tp, fp, fn, tn = (counts[k] for k in ("tp", "fp", "fn", "tn"))
    precision = tp / (tp + fp) if tp + fp else None
    recall = tp / (tp + fn) if tp + fn else None
    f1 = 2 * tp / (2 * tp + fp + fn) if tp + fp + fn else None
    return {
        "qwen": tp + fn,
        "model": tp + fp,
        "both": tp,
        "agree": (tp + tn) / max(1, tp + fp + fn + tn),
        "precision": precision,
        "recall": recall,
        "f1": f1,
    }


def disagreements(truth, preds):
    rows = []
    for sha in sorted(preds):
        probs, qwen = preds[sha], truth[sha]
        rows += [(sha, c, "model only", probs[c]) for c in sorted(set(probs) - qwen)]
        rows += [(sha, c, "qwen only", None) for c in sorted(qwen - set(probs))]
    return rows


def sample_disagreements(rows, count, seed):
    picked = random.Random(seed).sample(rows, min(count, len(rows)))
    return sorted(picked, key=lambda r: (r[0], r[1]))


def tally(rows):
    table = defaultdict(Counter)
    for r in rows:
        if r["verdict"] in VERDICTS:
            table[r["side"]][r["verdict"]] += 1
    return {side: dict(c) for side, c in table.items()}


def read_csv(path):
    with open(path) as f:
        return list(csv.DictReader(f))


def write_csv(path, rows, fields):
    with open(path, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        w.writerows(rows)


def sheet(rows, blobs, out, cols=5, cell=256):
    from PIL import Image, ImageDraw, ImageOps

    text_h = 28
    n_rows = -(-len(rows) // cols)
    img = Image.new("RGB", (cols * cell, n_rows * (cell + text_h)), "white")
    draw = ImageDraw.Draw(img)
    for i, r in enumerate(rows):
        x, y = (i % cols) * cell, (i // cols) * (cell + text_h)
        try:
            pic = ImageOps.exif_transpose(Image.open(blobs / r["sha256"])).convert("RGB")
            pic.thumbnail((cell - 4, cell - 4))
            img.paste(pic, (x + 2, y + 2))
        except Exception:
            draw.text((x + 4, y + 20), "unreadable", fill="red")
        draw.rectangle((x + 2, y + 2, x + 34, y + 14), fill="black")
        draw.text((x + 4, y + 3), str(r["index"]), fill="yellow")
        side = "M" if r["side"] == "model only" else "Q"
        draw.text((x + 4, y + cell), f"{side}: {r['class']}"[:44], fill="black")
    img.save(out)


def cmd_pick(args):
    taxonomy = json.loads((HERE / "taxonomy.json").read_text())
    lookup = class_lookup(taxonomy)
    raw = kubectl(
        args.namespace,
        args.db_deploy,
        "sh",
        "-c",
        'exec mariadb -uroot -p"$MARIADB_ROOT_PASSWORD" route96 -N -B -e "$0"',
        QUERY.format(model=args.model),
    ).decode()
    labelled = parse_labelled(raw)
    with open(Path(args.data) / "manifest.csv") as f:
        trained = {r["sha256"] for r in csv.DictReader(f)}
    out = Path(args.out)
    blobs = out / "blobs"
    blobs.mkdir(parents=True, exist_ok=True)
    chosen = pick(labelled, lookup, trained, args.count + args.spare, args.seed)
    for storage in args.storage_dir:
        todo = [s for s in chosen if not (blobs / s).exists()]
        source = argparse.Namespace(**{**vars(args), "storage_dir": storage})
        for i in range(0, len(todo), args.batch):
            for sha, data in fetch_batch(source, todo[i : i + args.batch]):
                (blobs / sha).write_bytes(data)
    kept = [s for s in chosen if (blobs / s).exists()][: args.count]
    for extra in set(p.name for p in blobs.iterdir()) - set(kept):
        (blobs / extra).unlink()
    rows = [{"sha256": s, "qwen": "|".join(labels_to_classes(labelled[s], lookup)), "llm_labels": "|".join(labelled[s])} for s in kept]
    write_csv(out / "heldout.csv", rows, ["sha256", "qwen", "llm_labels"])
    print(f"{len(labelled)} labelled, {len(trained)} in training, {len(kept)} held out written to {out}")


def fmt(v):
    return "" if v is None else f"{v:.2f}"


def cmd_compare(args):
    out = Path(args.out)
    taxonomy = json.loads((HERE / "taxonomy.json").read_text())
    classes = [c["name"] for c in taxonomy["classes"]]
    groups = {c["name"]: c["group"] for c in taxonomy["classes"]}
    truth = {r["sha256"]: set(filter(None, r["qwen"].split("|"))) for r in read_csv(out / "heldout.csv")}
    preds = {s: p for s, p in parse_predictions(args.predictions).items() if s in truth}
    table = agreement(truth, preds, classes)
    per_class = [{"class": c, "group": groups[c], **scores(table[c])} for c in classes]
    write_csv(out / "agreement.csv", [{k: fmt(v) if isinstance(v, float) else v for k, v in r.items()} for r in per_class], list(per_class[0]))
    total = Counter()
    for c in classes:
        total.update(table[c])
    micro = scores(total)
    active = [r for r in per_class if r["qwen"] or r["model"]]
    macro = sum(r["f1"] or 0 for r in active) / max(1, len(active))
    exact = sum(set(preds[s]) == truth[s] for s in preds)
    rows = disagreements(truth, preds)
    write_csv(out / "disagreements.csv", [{"sha256": s, "class": c, "side": side, "probability": fmt(p)} for s, c, side, p in rows], ["sha256", "class", "side", "probability"])
    sample = sample_disagreements(rows, args.sample, args.seed)
    sample_rows = [{"index": i, "sha256": s, "class": c, "side": side, "probability": fmt(p), "verdict": ""} for i, (s, c, side, p) in enumerate(sample)]
    write_csv(out / "graded_sample.csv", sample_rows, SAMPLE_FIELDS)
    for i in range(0, len(sample_rows), args.per_sheet):
        sheet(sample_rows[i : i + args.per_sheet], out / "blobs", out / f"sheet_{i // args.per_sheet:02d}.png")
    by_side = Counter(r[2] for r in rows)
    summary = {
        "images": len(preds),
        "micro_precision": micro["precision"],
        "micro_recall": micro["recall"],
        "micro_f1": micro["f1"],
        "macro_f1_classes_seen": macro,
        "classes_seen": len(active),
        "exact_match_images": exact,
        "qwen_labels": total["tp"] + total["fn"],
        "model_labels": total["tp"] + total["fp"],
        "agreed_labels": total["tp"],
        "model_only": by_side["model only"],
        "qwen_only": by_side["qwen only"],
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=1))
    print(json.dumps(summary, indent=1))


def apply_grades(rows, grades):
    codes = {"m": "model", "q": "qwen", "u": "unclear"}
    for token in grades.split():
        index, code = token[:-1], token[-1]
        for r in rows:
            if str(r["index"]) == index:
                r["verdict"] = codes[code]
    return rows


def cmd_report(args):
    rows = read_csv(args.sample)
    print("| disagreement | graded | model right | qwen right | unclear |\n|---|---|---|---|---|")
    for side, c in sorted(tally(rows).items()):
        n = sum(c.values())
        print(f"| {side} | {n} | {c.get('model', 0)} | {c.get('qwen', 0)} | {c.get('unclear', 0)} |")


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("pick")
    a.add_argument("--count", type=int, default=500)
    a.add_argument("--spare", type=int, default=60)
    a.add_argument("--seed", type=int, default=1509)
    a.add_argument("--data", default=str(DATA))
    a.add_argument("--namespace", default="nostr-download")
    a.add_argument("--app-deploy", default="app")
    a.add_argument("--db-deploy", default="mysql")
    a.add_argument("--model", default="qwen3.5")
    a.add_argument("--storage-dir", action="append", default=None)
    a.add_argument("--batch", type=int, default=100)
    b = sub.add_parser("compare")
    b.add_argument("predictions")
    b.add_argument("--sample", type=int, default=100)
    b.add_argument("--per-sheet", type=int, default=20)
    b.add_argument("--seed", type=int, default=1509)
    c = sub.add_parser("report")
    c.add_argument("sample")
    g = sub.add_parser("grade")
    g.add_argument("sample")
    g.add_argument("grades")
    for s in (a, b):
        s.add_argument("--out", default=str(HELDOUT))
    args = p.parse_args()
    if args.cmd == "pick":
        args.storage_dir = args.storage_dir or ["/data", "/cold"]
        cmd_pick(args)
    elif args.cmd == "compare":
        cmd_compare(args)
    elif args.cmd == "grade":
        rows = apply_grades(read_csv(args.sample), args.grades)
        write_csv(args.sample, rows, SAMPLE_FIELDS)
        cmd_report(args)
    else:
        cmd_report(args)


if __name__ == "__main__":
    main()
