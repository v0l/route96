import argparse
import csv
import re
from pathlib import Path

import torch
from torch.utils.data import DataLoader

from compare import NSFW_LABELS, Images, load
from train import read_manifest

GROUPS = {
    "clock": r"clock|watch|timepiece|\bdial\b",
    "anime": r"anime|manga|cartoon|chibi|waifu|illustration",
}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--data", default=str(Path(__file__).parent / "data"))
    p.add_argument("--models", nargs="+", required=True)
    p.add_argument("--out", default="probe.csv")
    args = p.parse_args()
    data = Path(args.data)
    labels = {r["sha256"]: r["llm_labels"].replace("_", " ") for r in csv.DictReader(open(data / "manifest.csv"))}
    rows = read_manifest(data)
    device = "cuda" if torch.cuda.is_available() else "cpu"
    selected = {g: [r for r in rows if re.search(p, labels[r[0]], re.I)] for g, p in GROUPS.items()}
    union = {r[0]: r for rs in selected.values() for r in rs}
    union_rows = list(union.values())
    scores = {}
    for name in args.models:
        model, forward, size, mean, std, id2label = load(name)
        model = model.to(device)
        picked = [i for i, l in id2label.items() if l in NSFW_LABELS]
        loader = DataLoader(Images(data, union_rows, size, mean, std), batch_size=64, num_workers=8)
        out = []
        with torch.no_grad():
            for x, _ in loader:
                out.append(torch.softmax(forward(x.to(device)).float(), -1).cpu()[:, picked].sum(-1))
        scores[name] = dict(zip(union.keys(), torch.cat(out).tolist()))
    with open(args.out, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["sha256", "group", "truth", *args.models])
        for g, rs in selected.items():
            for r in rs:
                w.writerow([r[0], g, "nsfw" if r[1] else "normal", *[round(scores[m][r[0]], 4) for m in args.models]])
    for g, rs in selected.items():
        sfw = [r for r in rs if not r[1]]
        print(f"{g}: {len(rs)} images, {len(sfw)} SFW")
        for m in args.models:
            fp = sum(1 for r in sfw if scores[m][r[0]] >= 0.5)
            print(f"  {m:45} false positives at 0.5: {fp}/{len(sfw)} ({100*fp/max(len(sfw),1):.1f}%)")


if __name__ == "__main__":
    main()
