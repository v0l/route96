import argparse
import csv
import json
import random
import time
from pathlib import Path

import numpy as np
import torch

from floors import BLOCKED_KINDS, dropped_labels, normalized_labels, read_label_map
from multilabel import best_thresholds
from workdir import DATA, OUT, ROOT

HELDOUT = ROOT / "data" / "heldout"
OPEN = OUT / "openvocab"
BASE = "google/siglip2-base-patch16-224"
TEMPLATES = {"bare": "{}", "photo": "a photo of {}.", "image": "an image of {}."}


def vocabulary(kinds, dropped):
    return sorted(l for l, k in kinds.items() if k not in BLOCKED_KINDS and l not in dropped)


def fill(template, label):
    return TEMPLATES[template].format(label)


def embed_texts(texts, batch=256, threads=8):
    from transformers import AutoModel, AutoTokenizer

    torch.set_num_threads(threads)
    tok = AutoTokenizer.from_pretrained(BASE)
    model = AutoModel.from_pretrained(BASE).eval()
    out = []
    started = time.time()
    with torch.no_grad():
        for i in range(0, len(texts), batch):
            ids = tok([t.lower() for t in texts[i : i + batch]], padding="max_length", max_length=64, truncation=True, return_tensors="pt")
            out.append(model.get_text_features(input_ids=ids["input_ids"]).float())
    seconds = time.time() - started
    return torch.nn.functional.normalize(torch.cat(out), dim=-1), seconds, float(model.logit_scale.exp()), float(model.logit_bias)


def scores(image_feats, text_feats, scale, bias):
    return torch.sigmoid(torch.nn.functional.normalize(image_feats, dim=-1) @ text_feats.T * scale + bias)


def truth_matrix(label_sets, names):
    index = {n: i for i, n in enumerate(names)}
    t = torch.zeros(len(label_sets), len(names))
    for r, labels in enumerate(label_sets):
        for l in labels:
            if l in index:
                t[r, index[l]] = 1.0
    return t


def pick(probs, threshold, top_k):
    keep = probs >= threshold
    if top_k:
        ranks = probs.topk(min(top_k, probs.shape[1]), dim=1).indices
        top = torch.zeros_like(keep)
        top.scatter_(1, ranks, True)
        keep &= top
    return keep


def label_sets(pred, names):
    return [{names[j] for j in torch.nonzero(row).flatten().tolist()} for row in pred]


def agreement(pred_sets, truth_sets):
    tp = sum(len(p & t) for p, t in zip(pred_sets, truth_sets))
    n_pred = sum(len(p) for p in pred_sets)
    n_true = sum(len(t) for t in truth_sets)
    precision = tp / n_pred if n_pred else 0.0
    recall = tp / n_true if n_true else 0.0
    f1 = 2 * precision * recall / (precision + recall) if precision + recall else 0.0
    return {"qwen": n_true, "model": n_pred, "both": tp, "precision": round(precision, 4), "recall": round(recall, 4), "f1": round(f1, 4), "per_image": round(n_pred / max(1, len(pred_sets)), 2)}


def tensor_agreement(pred, truth, extra_truth=0):
    tp = float((pred & truth).sum())
    n_pred, n_true = float(pred.sum()), float(truth.sum()) + extra_truth
    precision = tp / n_pred if n_pred else 0.0
    recall = tp / n_true if n_true else 0.0
    f1 = 2 * precision * recall / (precision + recall) if precision + recall else 0.0
    return {"qwen": int(n_true), "model": int(n_pred), "both": int(tp), "precision": round(precision, 4), "recall": round(recall, 4), "f1": round(f1, 4), "per_image": round(n_pred / max(1, pred.shape[0]), 2)}


def tune(probs, truth, grid, top_ks, base=None, covered=None, extra_truth=0):
    best = (None, 0.0, 0)
    for k in top_ks:
        for t in grid:
            pred = pick(probs, float(t), k)
            if base is not None and covered is not None:
                pred = base | (pred & ~covered)
            a = tensor_agreement(pred, truth, extra_truth)
            if best[0] is None or a["f1"] > best[0]["f1"]:
                best = (a, float(t), k)
    return best


def hybrid(head_sets, zero_sets, head_names):
    covered = set(head_names)
    return [h | {z for z in zs if z not in covered} for h, zs in zip(head_sets, zero_sets)]


def head_probs(feats, head_path):
    from safetensors import safe_open
    from safetensors.torch import load_file

    t = load_file(head_path)
    with safe_open(head_path, "pt") as f:
        meta = f.metadata()
    names = json.loads(meta["classes"])
    return torch.sigmoid(feats @ t["weight"].T + t["bias"]), t["thresholds"], names


def head_sets(probs, thresholds, names):
    return label_sets(probs >= thresholds, names)


def cmd_embed(args):
    mapping, kinds = read_label_map()
    names = vocabulary(kinds, dropped_labels())
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    for template in args.templates:
        feats, seconds, scale, bias = embed_texts([fill(template, n) for n in names], threads=args.threads)
        torch.save({"names": names, "feats": feats, "scale": scale, "bias": bias, "seconds": seconds, "threads": args.threads}, out / f"text_{template}.pt")
        print(json.dumps({"template": template, "labels": len(names), "seconds": round(seconds, 1), "threads": args.threads}), flush=True)


def cmd_features(args):
    from PIL import Image, ImageOps
    from transformers import SiglipVisionModel

    from train import eval_transform

    torch.set_num_threads(args.threads)
    model = SiglipVisionModel.from_pretrained(BASE).eval()
    rows = list(csv.DictReader(open(Path(args.heldout) / "heldout.csv")))
    shas, feats = [], []
    with torch.no_grad():
        for r in rows:
            try:
                img = ImageOps.exif_transpose(Image.open(Path(args.heldout) / "blobs" / r["sha256"])).convert("RGB")
            except Exception:
                continue
            if min(img.size) < 32:
                continue
            feats.append(model(pixel_values=eval_transform(img).unsqueeze(0)).pooler_output[0])
            shas.append(r["sha256"])
    torch.save({"shas": shas, "feats": torch.stack(feats)}, Path(args.out) / "heldout_features.pt")
    print(f"{len(shas)} of {len(rows)} held out images embedded")


def manifest_truth(data, mapping):
    rows = [r for r in csv.DictReader(open(Path(data) / "manifest.csv")) if r["split"] == "val"]
    return [normalized_labels(r["llm_labels"].split("|"), mapping) for r in rows]


def head_in_vocabulary(head_pred, head_names, names):
    index = {n: i for i, n in enumerate(names)}
    out = torch.zeros(head_pred.shape[0], len(names), dtype=torch.bool)
    covered = torch.zeros(len(names), dtype=torch.bool)
    for j, n in enumerate(head_names):
        if n not in index:
            continue
        out[:, index[n]] = head_pred[:, j]
        covered[index[n]] = True
    return out, covered


def cmd_evaluate(args):
    mapping, kinds = read_label_map()
    out = Path(args.out)
    cache = torch.load(args.features)
    val_feats = cache[3]
    val_truth = manifest_truth(args.data, mapping)
    held = torch.load(out / "heldout_features.pt")
    rows = {r["sha256"]: r for r in csv.DictReader(open(Path(args.heldout) / "heldout.csv"))}
    held_truth = [normalized_labels(rows[s]["llm_labels"].split("|"), mapping) for s in held["shas"]]
    grid = torch.logspace(-4, -0.3, 60)
    top_ks = [0, 3, 5, 10]
    hp_val, h_thr, h_names = head_probs(val_feats, args.head)
    hp_held, _, _ = head_probs(held["feats"], args.head)
    head_val, head_held = head_sets(hp_val, h_thr, h_names), head_sets(hp_held, h_thr, h_names)
    result = {"val_images": len(val_truth), "heldout_images": len(held_truth), "head": {"val": agreement(head_val, val_truth), "heldout": agreement(head_held, held_truth)}, "zero_shot": {}, "hybrid": {}}
    keep = {}
    for template in args.templates:
        text = torch.load(out / f"text_{template}.pt")
        names = text["names"]
        vocab = set(names)
        val_t = truth_matrix(val_truth, names) > 0
        extra = sum(len(x - vocab) for x in val_truth)
        zp_val = scores(val_feats, text["feats"], text["scale"], text["bias"])
        zp_held = scores(held["feats"], text["feats"], text["scale"], text["bias"])
        a, t, k = tune(zp_val, val_t, grid, top_ks, extra_truth=extra)
        z_held = label_sets(pick(zp_held, t, k), names)
        result["zero_shot"][template] = {"threshold": t, "top_k": k, "val": a, "heldout": agreement(z_held, held_truth), "labels": len(names)}
        base_val, covered = head_in_vocabulary(hp_val >= h_thr, h_names, names)
        ag, tt, kk = tune(zp_val, val_t, grid, top_ks, base=base_val, covered=covered, extra_truth=extra)
        h_held = hybrid(head_held, label_sets(pick(zp_held, tt, kk), names), h_names)
        result["hybrid"][template] = {"threshold": tt, "top_k": kk, "val": ag, "heldout": agreement(h_held, held_truth)}
        keep[template] = (names, zp_held, t, k, tt, kk, h_held, z_held)
        print(json.dumps({"template": template, "zero_shot": result["zero_shot"][template], "hybrid": result["hybrid"][template]}), flush=True)
    best = max(args.templates, key=lambda t: result["hybrid"][t]["val"]["f1"])
    names, zp_held, t, k, tt, kk, h_held, z_held = keep[best]
    result["best_template"] = best
    covered = set(h_names)
    result["heldout_coverage"] = {
        "qwen_labels": sum(len(x) for x in held_truth),
        "qwen_labels_in_head_classes": sum(len(x & covered) for x in held_truth),
        "qwen_labels_in_vocabulary": sum(len(x & set(names)) for x in held_truth),
    }
    result["heldout_recall_outside_head"] = {
        "zero_shot": recall_outside(z_held, held_truth, covered),
        "hybrid": recall_outside(h_held, held_truth, covered),
    }
    write_predictions(out / "heldout_predictions.tsv", held["shas"], held_truth, head_held, z_held, h_held)
    extras = [(s, l, l in q) for s, h, q, hd in zip(held["shas"], h_held, held_truth, head_held) for l in sorted(h - hd)]
    write_sample(extras, args.sample, args.seed, out, Path(args.heldout) / "blobs")
    result["hybrid_zero_shot_extras"] = len(extras)
    (out / "summary.json").write_text(json.dumps(result, indent=1))
    print(json.dumps({k: result[k] for k in ("best_template", "heldout_coverage", "heldout_recall_outside_head", "hybrid_zero_shot_extras")}))


def recall_outside(pred_sets, truth_sets, covered):
    tp = sum(len((p & t) - covered) for p, t in zip(pred_sets, truth_sets))
    n = sum(len(t - covered) for t in truth_sets)
    return {"qwen": n, "found": tp, "recall": round(tp / max(1, n), 4)}


def write_predictions(path, shas, truth, head, zero, mixed):
    with open(path, "w") as f:
        f.write("sha256\tqwen\thead\tzero_shot\thybrid\n")
        for row in zip(shas, truth, head, zero, mixed):
            f.write(row[0] + "\t" + "\t".join("|".join(sorted(x)) for x in row[1:]) + "\n")


def cmd_fixture(args):
    from safetensors.torch import save_file
    from transformers import AutoModel, AutoTokenizer

    text = torch.load(Path(args.out) / f"text_{args.template}.pt")
    names = text["names"]
    picked = sorted(random.Random(args.seed).sample(range(len(names)), args.count))
    tok = AutoTokenizer.from_pretrained(BASE)
    model = AutoModel.from_pretrained(BASE).eval()
    ids = tok([fill(args.template, names[i]).lower() for i in picked], padding="max_length", max_length=64, truncation=True, return_tensors="pt")["input_ids"]
    with torch.no_grad():
        feats = model.get_text_features(input_ids=ids).float()
    tensors = {
        "input_ids": ids.to(torch.int64).contiguous(),
        "features": feats.contiguous(),
        "labels": text["feats"].contiguous(),
        "scale": torch.tensor([text["scale"]]),
        "bias": torch.tensor([text["bias"]]),
    }
    save_file(tensors, Path(args.out) / "candle_text_fixture.safetensors")
    print(f"{len(picked)} labels, {tensors['labels'].shape[0]} label vectors written")


def floor_sets(feats, path):
    saved = torch.load(path)
    w, b = saved["state"]["weight"], saved["state"]["bias"]
    probs = torch.sigmoid(feats @ w.T + b)
    thresholds = torch.tensor(saved["thresholds"])
    return label_sets(probs >= thresholds, saved["names"]), saved["names"]


def cmd_compare(args):
    mapping, kinds = read_label_map()
    out = Path(args.out)
    held = torch.load(out / "heldout_features.pt")
    rows = {r["sha256"]: r for r in csv.DictReader(open(Path(args.heldout) / "heldout.csv"))}
    truth = [normalized_labels(rows[s]["llm_labels"].split("|"), mapping) for s in held["shas"]]
    _, _, base_names = head_probs(held["feats"], args.head)
    covered = set(base_names)
    table = []
    for floor in args.floors:
        sets, names = floor_sets(held["feats"], Path(args.floors_dir) / f"floor_{floor}" / "head.pt")
        table.append({"route": f"trained head, floor {floor}", "classes": len(names), **agreement(sets, truth), "outside_328": recall_outside(sets, truth, covered)})
    hp, thr, h_names = head_probs(held["feats"], args.head)
    head = head_sets(hp, thr, h_names)
    text = torch.load(out / f"text_{args.template}.pt")
    zp = scores(held["feats"], text["feats"], text["scale"], text["bias"])
    for k in args.top_ks:
        for t in args.thresholds:
            sets = hybrid(head, label_sets(pick(zp, t, k), text["names"]), h_names)
            table.append({"route": f"hybrid, zero shot top {k} at {t}", "classes": len(text["names"]), **agreement(sets, truth), "outside_328": recall_outside(sets, truth, covered)})
    (out / "compare.json").write_text(json.dumps(table, indent=1))
    for r in table:
        print(json.dumps(r))
    sets, _ = floor_sets(held["feats"], Path(args.floors_dir) / f"floor_{min(args.floors)}" / "head.pt")
    new = [(s, l, l in q) for s, p, q in zip(held["shas"], sets, truth) for l in sorted(p - covered)]
    write_sample(new, args.sample, args.seed, out / "floor_sample", Path(args.heldout) / "blobs")


def write_sample(pairs, count, seed, target, blobs):
    from heldout import SAMPLE_FIELDS, sheet, write_csv

    target.mkdir(parents=True, exist_ok=True)
    sample = sorted(random.Random(seed).sample(pairs, min(count, len(pairs))))
    rows = [{"index": i, "sha256": s, "class": l, "side": "qwen too" if agreed else "model only", "probability": "", "verdict": ""} for i, (s, l, agreed) in enumerate(sample)]
    write_csv(target / "graded_sample.csv", rows, SAMPLE_FIELDS)
    for i in range(0, len(rows), 20):
        sheet(rows[i : i + 20], blobs, target / f"sheet_{i // 20:02d}.png")


def cmd_bench(args):
    text = torch.load(Path(args.out) / f"text_{args.template}.pt")
    torch.set_num_threads(args.threads)
    feats = torch.randn(1, text["feats"].shape[1])
    for _ in range(5):
        scores(feats, text["feats"], text["scale"], text["bias"])
    started = time.time()
    for _ in range(args.rounds):
        scores(feats, text["feats"], text["scale"], text["bias"])
    print(json.dumps({"labels": len(text["names"]), "ms_per_image": round(1000 * (time.time() - started) / args.rounds, 3), "threads": args.threads, "embed_seconds": text["seconds"], "embed_threads": text["threads"]}))


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    e = sub.add_parser("embed")
    e.add_argument("--templates", nargs="+", default=list(TEMPLATES))
    f = sub.add_parser("features")
    v = sub.add_parser("evaluate")
    v.add_argument("--templates", nargs="+", default=list(TEMPLATES))
    v.add_argument("--head", default=str(OUT / "siglip_plain" / "head.safetensors"))
    v.add_argument("--sample", type=int, default=100)
    v.add_argument("--seed", type=int, default=1539)
    b = sub.add_parser("bench")
    b.add_argument("--template", default="photo")
    b.add_argument("--rounds", type=int, default=200)
    x = sub.add_parser("fixture")
    x.add_argument("--template", default="photo")
    x.add_argument("--count", type=int, default=64)
    x.add_argument("--seed", type=int, default=1539)
    c = sub.add_parser("compare")
    c.add_argument("--head", default=str(OUT / "siglip_plain" / "head.safetensors"))
    c.add_argument("--floors-dir", default=str(OUT / "floors"))
    c.add_argument("--floors", type=int, nargs="+", default=[50, 30, 20, 10])
    c.add_argument("--template", default="bare")
    c.add_argument("--top-ks", type=int, nargs="+", default=[1, 3, 5])
    c.add_argument("--thresholds", type=float, nargs="+", default=[0.0001, 0.001, 0.01, 0.1])
    c.add_argument("--sample", type=int, default=40)
    c.add_argument("--seed", type=int, default=1539)
    for s in (e, f, v, b, x, c):
        s.add_argument("--out", default=str(OPEN))
        s.add_argument("--threads", type=int, default=8)
        s.add_argument("--data", default=str(DATA))
        s.add_argument("--heldout", default=str(HELDOUT))
        s.add_argument("--features", default=str(OUT / "siglip_features_3.pt"))
    args = p.parse_args()
    Path(args.out).mkdir(parents=True, exist_ok=True)
    {"embed": cmd_embed, "features": cmd_features, "evaluate": cmd_evaluate, "bench": cmd_bench, "fixture": cmd_fixture, "compare": cmd_compare}[args.cmd](args)


if __name__ == "__main__":
    main()
