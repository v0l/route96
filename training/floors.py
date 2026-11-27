import argparse
import csv
import json
import time
from collections import Counter, defaultdict
from pathlib import Path

import torch

from taxonomy import DECISIONS, HERE, IMAGE_MIMES, read_merges
from workdir import DATA, LABELS, OUT, ROOT

NORMALIZED = ROOT / "data" / "normalized"
FLOORS = [50, 30, 20, 10]
BLOCKED_KINDS = {"age", "age and adult"}


class Space:
    def __init__(self, names):
        self.names = names
        self.index = {n: i for i, n in enumerate(names)}

    def __len__(self):
        return len(self.names)

    def vector(self, labels):
        v = torch.zeros(len(self.names))
        for n in labels:
            if n in self.index:
                v[self.index[n]] = 1.0
        return v


def read_label_map(path=NORMALIZED / "label_map.tsv"):
    mapping, kinds = {}, {}
    with open(path) as f:
        for r in csv.DictReader(f, delimiter="\t", quoting=csv.QUOTE_NONE):
            if r["normalized"]:
                mapping[r["raw"]] = r["normalized"]
                kinds[r["normalized"]] = r["kind"]
    return mapping, kinds


def file_counts(labels_path, mapping, mimes=IMAGE_MIMES):
    files = defaultdict(set)
    with open(labels_path) as f:
        for line in f:
            parts = line.rstrip("\n").split("\t")
            if len(parts) == 3 and parts[1].split(";")[0].strip() in mimes and mapping.get(parts[2]):
                files[parts[0]].add(mapping[parts[2]])
    counts = Counter()
    for labels in files.values():
        counts.update(labels)
    return counts


def dropped_labels(path=DECISIONS):
    return {label for label, (cls, _) in read_merges(path).items() if cls == "-"}


def floor_classes(counts, base, floor, kinds, dropped):
    extra = sorted(
        (l for l, n in counts.items() if n >= floor and l not in base and l not in dropped and kinds.get(l, "") not in BLOCKED_KINDS),
        key=lambda l: (-counts[l], l),
    )
    return list(base) + extra


def normalized_labels(raw_labels, mapping):
    return {mapping[l] for l in raw_labels if mapping.get(l)}


def tier_of(cls, counts, base, floors=FLOORS):
    if cls in base:
        return "base"
    for f in floors:
        if counts[cls] >= f:
            return f"{f}+"
    return None


def subset_scores(per_class, names):
    rows = [r for r in per_class if r["class"] in names and r["support"] > 0]
    if not rows:
        return {"classes": 0, "with_val": 0, "macro_f1": None, "macro_precision": None, "macro_recall": None}
    n = len(rows)
    return {
        "classes": len(names),
        "with_val": n,
        "macro_f1": round(sum(r["f1"] for r in rows) / n, 4),
        "macro_precision": round(sum(r["precision"] for r in rows) / n, 4),
        "macro_recall": round(sum(r["recall"] for r in rows) / n, 4),
    }


def train_head(feats, truth, val_feats, val_truth, epochs, batch, lr, device, seed=0):
    torch.manual_seed(seed)
    head = torch.nn.Linear(feats.shape[1], truth.shape[1]).to(device)
    optimizer = torch.optim.AdamW(head.parameters(), lr=lr, weight_decay=1e-4)
    steps = epochs * (len(feats) // batch)
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, steps)
    feats, truth = feats.to(device), truth.to(device)
    val_feats = val_feats.to(device)
    best = None
    for epoch in range(epochs):
        head.train()
        order = torch.randperm(len(feats), device=device)
        for i in range(0, len(order) - batch + 1, batch):
            idx = order[i : i + batch]
            loss = torch.nn.functional.binary_cross_entropy_with_logits(head(feats[idx]), truth[idx])
            optimizer.zero_grad()
            loss.backward()
            optimizer.step()
            scheduler.step()
        head.eval()
        with torch.no_grad():
            probs = torch.sigmoid(head(val_feats)).cpu()
        yield epoch, head, probs


def run_floor(args, floor, counts, base, kinds, dropped, mapping, cache, rows):
    from train_taxonomy import report

    names = floor_classes(counts, base, floor, kinds, dropped)
    space = Space(names)
    feats, _, _, val_feats, _, _ = cache
    train_rows = [r for r in rows if r["split"] == "train"]
    val_rows = [r for r in rows if r["split"] == "val"]

    def truth_of(rs):
        return torch.stack([space.vector(set(filter(None, r["positives"].split("|"))) | normalized_labels(r["llm_labels"].split("|"), mapping)) for r in rs])

    truth = truth_of(train_rows)
    passes = feats.shape[0] // truth.shape[0]
    truth = truth.repeat(passes, 1)
    val_truth = truth_of(val_rows)
    started = time.time()
    best = None
    for epoch, head, probs in train_head(feats, truth, val_feats, val_truth, args.epochs, args.batch, args.lr, args.device):
        r = report(probs, val_truth, torch.zeros_like(val_truth), space)
        if best is None or r["macro_f1"] > best[0]["macro_f1"]:
            best = (r, {k: v.detach().cpu().clone() for k, v in head.state_dict().items()}, probs)
    seconds = time.time() - started
    if best is None:
        raise ValueError("no epochs")
    r, state, probs = best
    tiers = defaultdict(set)
    for n in names:
        tiers[tier_of(n, counts, set(base))].add(n)
    out = {
        "floor": floor,
        "classes": len(names),
        "seconds": round(seconds, 1),
        "macro_f1": r["macro_f1"],
        "micro_f1": r["micro_f1"],
        "top5_hit": r["top5_hit"],
        "fitted_on_val_macro_f1": r["fitted_on_val_macro_f1"],
        "tiers": {t: subset_scores(r["per_class"], ns) for t, ns in sorted(tiers.items())},
    }
    target = Path(args.out) / f"floor_{floor}"
    target.mkdir(parents=True, exist_ok=True)
    torch.save({"names": names, "state": state, "thresholds": r["thresholds"], "probs": probs, "val_shas": [x["sha256"] for x in val_rows]}, target / "head.pt")
    with open(target / "per_class.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=["class", "tier", "images", "support", "predicted", "precision", "recall", "f1", "threshold"])
        w.writeheader()
        for c in r["per_class"]:
            w.writerow({**c, "tier": tier_of(c["class"], counts, set(base)), "images": counts[c["class"]]})
    (target / "report.json").write_text(json.dumps(out, indent=1))
    return out


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--data", default=str(DATA))
    p.add_argument("--labels", default=str(LABELS / "labels.tsv"))
    p.add_argument("--features", default=str(OUT / "siglip_features_3.pt"))
    p.add_argument("--out", default=str(OUT / "floors"))
    p.add_argument("--floors", type=int, nargs="+", default=FLOORS)
    p.add_argument("--epochs", type=int, default=40)
    p.add_argument("--batch", type=int, default=64)
    p.add_argument("--lr", type=float, default=1e-3)
    p.add_argument("--threads", type=int, default=8)
    p.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    args = p.parse_args()
    torch.set_num_threads(args.threads)
    mapping, kinds = read_label_map()
    counts = file_counts(args.labels, mapping)
    base = [c["name"] for c in json.loads((HERE / "taxonomy.json").read_text())["classes"]]
    dropped = dropped_labels()
    cache = torch.load(args.features)
    with open(Path(args.data) / "manifest.csv") as f:
        rows = list(csv.DictReader(f))
    results = []
    for floor in args.floors:
        results.append(run_floor(args, floor, counts, base, kinds, dropped, mapping, cache, rows))
        print(json.dumps(results[-1]), flush=True)
    Path(args.out).mkdir(parents=True, exist_ok=True)
    (Path(args.out) / "summary.json").write_text(json.dumps(results, indent=1))


if __name__ == "__main__":
    main()
