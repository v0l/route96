import argparse
import json
from pathlib import Path

import torch
from PIL import Image
from torch.utils.data import DataLoader, Dataset
from huggingface_hub import hf_hub_download
from transformers import AutoModelForImageClassification

from train import (
    best_threshold,
    hard_negative_fpr,
    metrics,
    read_manifest,
    split,
    split_by_shas,
    val_shas_from,
)

NSFW_LABELS = {
    "nsfw", "porn", "pornography", "hentai", "sexy", "enticingorsensual",
    "explicit", "sexual", "suggestive", "unsafe", "questionable", "low", "medium", "high",
}
STRICT_LABELS = {
    "nsfw", "porn", "pornography", "hentai", "explicit", "sexual", "unsafe",
    "medium", "high",
}

DEFAULT_MODELS = [
    "out/model",
    "Falconsai/nsfw_image_detection",
    "umairrkhn/fine-tuned-nsfw-classification",
    "giacomoarienti/nsfw-classifier",
    "LukeJacob2023/nsfw-image-detector",
    "AdamCodd/vit-base-nsfw-detector",
    "Parveshiiii/SafeScan",
    "carbon225/vit-base-patch16-224-hentai",
    "JanadaSroor/vit-nsfw-classifier",
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
        sha, label, _ = self.rows[i]
        img = Image.open(self.dir / f"{sha}.jpg").convert("RGB").resize(
            (self.size, self.size), Image.BILINEAR
        )
        arr = torch.frombuffer(bytearray(img.tobytes()), dtype=torch.uint8)
        arr = arr.view(self.size, self.size, 3).permute(2, 0, 1).float() / 255.0
        return (arr - self.mean) / self.std, label


def load(name):
    if name.startswith("Marqo/"):
        import timm

        model = timm.create_model(f"hf-hub:{name}", pretrained=True).eval()
        cfg = model.pretrained_cfg
        labels = {i: l.lower() for i, l in enumerate(cfg["label_names"])}
        return model, lambda x: model(x), cfg["input_size"][1], cfg["mean"], cfg["std"], labels
    model = AutoModelForImageClassification.from_pretrained(name).eval()
    labels = {int(k): v.lower() for k, v in model.config.id2label.items()}
    if hasattr(model, "timm_model"):
        cfg = model.timm_model.pretrained_cfg
        size, mean, std = cfg["input_size"][1], cfg["mean"], cfg["std"]
    elif Path(name).exists():
        size, mean, std = model.config.image_size, [0.5] * 3, [0.5] * 3
    else:
        pre = json.load(open(hf_hub_download(name, "preprocessor_config.json")))
        size = pre["size"] if isinstance(pre["size"], int) else pre["size"].get("height", model.config.image_size)
        mean, std = pre["image_mean"], pre["image_std"]
    return model, lambda x: model(pixel_values=x).logits, size, mean, std, labels


def average_precision(probs, labels):
    order = torch.argsort(probs, descending=True)
    hits = (labels[order] == 1).float()
    precision_at_k = hits.cumsum(0) / torch.arange(1, len(hits) + 1)
    return (precision_at_k * hits).sum().item() / hits.sum().item()


@torch.no_grad()
def score(name, args, rows, device):
    model, forward, size, mean, std, id2label = load(name)
    model = model.to(device)
    loader = DataLoader(
        Images(Path(args.data), rows, size, mean, std),
        batch_size=32,
        num_workers=args.workers,
    )
    variants = {"nsfw": NSFW_LABELS, "strict": STRICT_LABELS}
    picked = {
        k: [i for i, l in id2label.items() if l in v] for k, v in variants.items()
    }
    out = {k: [] for k in variants}
    labels = []
    for x, y in loader:
        p = torch.softmax(forward(x.to(device)).float(), -1).cpu()
        for k, idx in picked.items():
            out[k].append(p[:, idx].sum(-1))
        labels.append(y)
    labels = torch.cat(labels)
    kinds = [r[2] for r in rows]
    result = {"image_size": size, "classes": list(id2label.values())}
    for k, v in out.items():
        probs = torch.cat(v)
        t = best_threshold(probs, labels)
        result[k] = {
            "ap": round(average_precision(probs, labels), 4),
            "at_0.5": metrics(probs, labels),
            "best_threshold": t,
            "at_best": metrics(probs, labels, t),
            "hard_negative_fpr_at_0.5": hard_negative_fpr(probs, kinds),
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
    p.add_argument("--all", action="store_true")
    p.add_argument("--val-from")
    p.add_argument("--out", default="compare.json")
    args = p.parse_args()

    device = "cuda" if torch.cuda.is_available() else "cpu"
    rows = read_manifest(Path(args.data))
    if args.all:
        val_rows = rows
    elif args.val_from:
        val_rows = split_by_shas(rows, val_shas_from(args.val_from, args.val_fraction, args.seed))[1]
    else:
        val_rows = split(rows, args.val_fraction, args.seed)[1]
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
                f"  best t={m['best_threshold']} F1 {m['at_best']['f1']:.3f}  hardneg FPR {m['hard_negative_fpr_at_0.5']}"
            )
    Path(args.out).write_text(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
