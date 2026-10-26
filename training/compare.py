import argparse
import json
from pathlib import Path

import torch
from PIL import Image
from torch.utils.data import DataLoader, Dataset
from huggingface_hub import hf_hub_download
from transformers import ViTForImageClassification

from train import best_threshold, metrics, read_manifest, split

NSFW_LABELS = {"nsfw", "porn", "hentai", "sexy", "explicit", "unsafe"}
STRICT_LABELS = {"nsfw", "porn", "hentai", "explicit", "unsafe"}

DEFAULT_MODELS = [
    "out/model",
    "Falconsai/nsfw_image_detection",
    "umairrkhn/fine-tuned-nsfw-classification",
    "giacomoarienti/nsfw-classifier",
    "LukeJacob2023/nsfw-image-detector",
    "AdamCodd/vit-base-nsfw-detector",
    "hakl36/vit-base-nsfw-detector",
]


class Images(Dataset):
    def __init__(self, data_dir, rows, size, mean, std):
        self.dir = data_dir / "images"
        self.rows = rows
        self.size = size
        self.mean = torch.tensor(mean).view(3, 1, 1)
        self.std = torch.tensor(std).view(3, 1, 1)

    def __len__(self):
        return len(self.rows)

    def __getitem__(self, i):
        sha, label = self.rows[i]
        img = Image.open(self.dir / f"{sha}.jpg").convert("RGB").resize(
            (self.size, self.size), Image.BILINEAR
        )
        arr = torch.frombuffer(bytearray(img.tobytes()), dtype=torch.uint8)
        arr = arr.view(self.size, self.size, 3).permute(2, 0, 1).float() / 255.0
        return (arr - self.mean) / self.std, label


def normalization(name):
    if Path(name).exists():
        return [0.5] * 3, [0.5] * 3
    cfg = json.load(open(hf_hub_download(name, "preprocessor_config.json")))
    return cfg["image_mean"], cfg["image_std"]


def average_precision(probs, labels):
    order = torch.argsort(probs, descending=True)
    hits = (labels[order] == 1).float()
    precision_at_k = hits.cumsum(0) / torch.arange(1, len(hits) + 1)
    return (precision_at_k * hits).sum().item() / hits.sum().item()


@torch.no_grad()
def score(name, args, rows, device):
    model = ViTForImageClassification.from_pretrained(name).to(device).eval()
    mean, std = normalization(name)
    size = model.config.image_size
    id2label = {int(k): v.lower() for k, v in model.config.id2label.items()}
    loader = DataLoader(
        Images(Path(args.data), rows, size, mean, std),
        batch_size=64,
        num_workers=args.workers,
    )
    variants = {"nsfw": NSFW_LABELS, "strict": STRICT_LABELS}
    picked = {
        k: [i for i, l in id2label.items() if l in v] for k, v in variants.items()
    }
    out = {k: [] for k in variants}
    labels = []
    for x, y in loader:
        p = torch.softmax(model(pixel_values=x.to(device)).logits.float(), -1).cpu()
        for k, idx in picked.items():
            out[k].append(p[:, idx].sum(-1))
        labels.append(y)
    labels = torch.cat(labels)
    result = {"image_size": size, "classes": list(id2label.values())}
    for k, v in out.items():
        probs = torch.cat(v)
        t = best_threshold(probs, labels)
        result[k] = {
            "ap": round(average_precision(probs, labels), 4),
            "at_0.5": metrics(probs, labels),
            "best_threshold": t,
            "at_best": metrics(probs, labels, t),
        }
    del model
    torch.cuda.empty_cache()
    return result


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--data", default=str(Path(__file__).parent / "data"))
    p.add_argument("--models", nargs="*", default=DEFAULT_MODELS)
    p.add_argument("--val-fraction", type=float, default=0.15)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--workers", type=int, default=8)
    p.add_argument("--out", default="compare.json")
    args = p.parse_args()

    device = "cuda" if torch.cuda.is_available() else "cpu"
    _, val_rows = split(read_manifest(Path(args.data)), args.val_fraction, args.seed)
    results = {}
    for name in args.models:
        try:
            results[name] = score(name, args, val_rows, device)
        except Exception as e:
            results[name] = {"error": str(e)[:200]}
            continue
        r = results[name]
        print(name, r["image_size"], r["classes"])
        for k in ("nsfw", "strict"):
            m = r[k]
            print(
                f"  {k:6} AP {m['ap']:.3f}  @0.5 P {m['at_0.5']['precision']:.3f} R {m['at_0.5']['recall']:.3f} F1 {m['at_0.5']['f1']:.3f}"
                f"  best t={m['best_threshold']} F1 {m['at_best']['f1']:.3f}"
            )
    Path(args.out).write_text(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
