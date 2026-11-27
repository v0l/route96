import argparse
import csv
import json
import time
from pathlib import Path

import torch
from PIL import Image
from safetensors.torch import load_file, save_file

from train import eval_transform
from train_taxonomy import BASES, SiglipHead, logits_of
from workdir import DATA

HERE = Path(__file__).parent


def load(kind, checkpoint):
    if kind == "vit":
        from transformers import ViTForImageClassification

        return ViTForImageClassification.from_pretrained(checkpoint).eval()
    cfg = json.loads((checkpoint / "config.json").read_text())
    model = SiglipHead(BASES["siglip"], len(cfg["id2label"]))
    tensors = load_file(checkpoint / "model.safetensors")
    model.backbone.vision_model.load_state_dict({k[len("vision_model."):]: v for k, v in tensors.items() if k.startswith("vision_model.")})
    model.classifier.load_state_dict({k[len("classifier."):]: v for k, v in tensors.items() if k.startswith("classifier.")})
    return model.eval()


def val_images(data_dir, count):
    with open(data_dir / "manifest.csv") as f:
        shas = sorted(r["sha256"] for r in csv.DictReader(f) if r["split"] == "val")[:count]
    return torch.stack([eval_transform(Image.open(data_dir / "images" / f"{s}.jpg").convert("RGB")) for s in shas])


@torch.no_grad()
def main():
    p = argparse.ArgumentParser()
    p.add_argument("kind", choices=sorted(BASES))
    p.add_argument("checkpoint")
    p.add_argument("--data", default=str(DATA))
    p.add_argument("--count", type=int, default=64)
    p.add_argument("--threads", type=int, default=8)
    p.add_argument("--out", required=True)
    args = p.parse_args()
    torch.set_num_threads(args.threads)
    model = load(args.kind, Path(args.checkpoint))
    x = val_images(Path(args.data), args.count)
    logits_of(model, x[:2])
    started = time.time()
    logits = logits_of(model, x)
    seconds = time.time() - started
    save_file({"x": x.contiguous(), "logits": logits.contiguous()}, args.out)
    print(json.dumps({"kind": args.kind, "device": "pytorch cpu", "threads": args.threads, "batch": len(x), "images_per_second": round(len(x) / seconds, 2), "images_per_second_per_core": round(len(x) / seconds / args.threads, 2)}))


if __name__ == "__main__":
    main()
