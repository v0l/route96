import argparse
import csv
from pathlib import Path

import torch
from torch.utils.data import DataLoader

from compare import NSFW_LABELS, Images, load
from train import read_manifest

MODELS = {
    "ours": "out/model",
    "falcon": "Falconsai/nsfw_image_detection",
    "nostrbuild": "nostrbuild/nsfw-image-detection-large",
}


@torch.no_grad()
def score(name, rows, data, device, workers):
    model, forward, size, mean, std, id2label = load(name)
    model = model.to(device)
    picked = [i for i, l in id2label.items() if l in NSFW_LABELS]
    loader = DataLoader(Images(data, rows, size, mean, std), batch_size=64, num_workers=workers)
    out = []
    for x, _ in loader:
        p = torch.softmax(forward(x.to(device)).float(), -1).cpu()
        out.append(p[:, picked].sum(-1))
    return torch.cat(out).tolist()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--data", default=str(Path(__file__).parent / "data"))
    p.add_argument("--workers", type=int, default=8)
    args = p.parse_args()
    data = Path(args.data)
    rows = read_manifest(data)
    device = "cuda" if torch.cuda.is_available() else "cpu"
    scores = {k: score(v, rows, data, device, args.workers) for k, v in MODELS.items()}
    with open(data / "scores.csv", "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["sha256", *MODELS])
        for i, r in enumerate(rows):
            w.writerow([r[0], *[round(scores[k][i], 4) for k in MODELS]])


if __name__ == "__main__":
    main()
