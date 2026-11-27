import argparse
import json
from pathlib import Path

import torch
from safetensors.torch import load_file, save_file

from workdir import OUT


def head_tensors(model_tensors, thresholds):
    return {
        "weight": model_tensors["classifier.weight"].float().contiguous(),
        "bias": model_tensors["classifier.bias"].float().contiguous(),
        "thresholds": torch.tensor(thresholds, dtype=torch.float32),
    }


def export(run_dir, out):
    checkpoint = run_dir / "best"
    cfg = json.loads((checkpoint / "config.json").read_text())
    names = [cfg["id2label"][str(i)] for i in range(len(cfg["id2label"]))]
    thresholds = json.loads((run_dir / "report.json").read_text())["best"]["thresholds"]
    tensors = head_tensors(load_file(checkpoint / "model.safetensors"), thresholds)
    if tensors["weight"].shape[0] != len(names) or len(thresholds) != len(names):
        raise ValueError("head, thresholds and class names disagree")
    save_file(tensors, out, metadata={"classes": json.dumps(names), "base": "google/siglip2-base-patch16-224"})
    return tensors["weight"].shape


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--run", default=str(OUT / "siglip_plain"))
    p.add_argument("--out")
    args = p.parse_args()
    run_dir = Path(args.run)
    out = Path(args.out or run_dir / "head.safetensors")
    print(f"{tuple(export(run_dir, out))} written to {out}")


if __name__ == "__main__":
    main()
