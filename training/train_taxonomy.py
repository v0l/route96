import argparse
import csv
import json
import random
import time
from pathlib import Path

import torch
from PIL import Image
from safetensors.torch import save_file
from torch.utils.data import DataLoader, Dataset

from multilabel import (
    LabelSpace,
    best_thresholds,
    evaluate,
    hard_negative_rate,
    partial_weights,
    plain_weights,
    weighted_bce,
)
from train import eval_transform, train_transform

HERE = Path(__file__).parent
BASES = {"vit": "google/vit-base-patch16-224", "siglip": "google/siglip2-base-patch16-224"}


def read_rows(data_dir, space):
    with open(data_dir / "manifest.csv") as f:
        rows = []
        for r in csv.DictReader(f):
            rows.append(
                (
                    r["sha256"],
                    r["split"],
                    space.vector(r["positives"].split("|")),
                    space.vector(r["hard_negatives"].split("|")),
                )
            )
    return [r for r in rows if r[1] == "train"], [r for r in rows if r[1] == "val"]


class Images(Dataset):
    def __init__(self, data_dir, rows, tf):
        self.dir = data_dir / "images"
        self.rows = rows
        self.tf = tf

    def __len__(self):
        return len(self.rows)

    def __getitem__(self, i):
        sha, _, pos, hard = self.rows[i]
        return self.tf(Image.open(self.dir / f"{sha}.jpg").convert("RGB")), pos, hard


def halves(n, seed=1):
    idx = list(range(n))
    random.Random(seed).shuffle(idx)
    return torch.tensor(sorted(idx[: n // 2])), torch.tensor(sorted(idx[n // 2 :]))


def cross_fit(probs, truth):
    a, b = halves(probs.shape[0])
    out = torch.zeros_like(probs, dtype=torch.bool)
    out[a] = probs[a] >= best_thresholds(probs[b], truth[b])
    out[b] = probs[b] >= best_thresholds(probs[a], truth[a])
    return out


def report(probs, truth, hard, space):
    thresholds = best_thresholds(probs, truth)
    fitted = evaluate(probs, truth, thresholds, space.names)
    held = cross_fit(probs, truth)
    held_report = evaluate(held.float(), truth, torch.full_like(thresholds, 0.5), space.names)
    held_report["top5_hit"] = fitted["top5_hit"]
    held_report["top5_recall"] = fitted["top5_recall"]
    held_report["hard_negative_rate"] = hard_negative_rate(probs, hard, thresholds)
    held_report["thresholds"] = [round(float(t), 2) for t in thresholds]
    held_report["fitted_on_val_macro_f1"] = fitted["macro_f1"]
    return held_report


def loss_weights(args, space, pos, hard, generator):
    if args.loss == "plain":
        return plain_weights(pos)
    return partial_weights(
        pos, hard, space.same_group(pos), generator=generator, far_samples=args.far_samples, far_weight=args.far_weight
    )


class SiglipHead(torch.nn.Module):
    def __init__(self, base, classes):
        super().__init__()
        from transformers import SiglipVisionModel

        self.backbone = SiglipVisionModel.from_pretrained(base)
        self.backbone.requires_grad_(False)
        self.classifier = torch.nn.Linear(self.backbone.config.hidden_size, classes)

    def features(self, x):
        return self.backbone(pixel_values=x).pooler_output

    def forward(self, x):
        return self.classifier(self.features(x).float())


def build_model(args, space):
    if args.model == "vit":
        from transformers import ViTForImageClassification

        return ViTForImageClassification.from_pretrained(
            args.base,
            num_labels=len(space),
            id2label=dict(enumerate(space.names)),
            label2id=space.index,
            problem_type="multi_label_classification",
            ignore_mismatched_sizes=True,
        )
    return SiglipHead(args.base, len(space))


def logits_of(model, x):
    out = model(x) if isinstance(model, SiglipHead) else model(pixel_values=x).logits
    return out.float()


@torch.no_grad()
def predict(model, loader, device):
    model.eval()
    probs, truth, hard = [], [], []
    for x, pos, neg in loader:
        with torch.autocast(device, dtype=torch.bfloat16, enabled=device == "cuda"):
            probs.append(torch.sigmoid(logits_of(model, x.to(device))).cpu())
        truth.append(pos)
        hard.append(neg)
    return torch.cat(probs), torch.cat(truth), torch.cat(hard)


@torch.no_grad()
def embed(model, loader, device):
    model.eval()
    feats, truth, hard = [], [], []
    for x, pos, neg in loader:
        with torch.autocast(device, dtype=torch.bfloat16, enabled=device == "cuda"):
            feats.append(model.features(x.to(device)).float().cpu())
        truth.append(pos)
        hard.append(neg)
    return torch.cat(feats), torch.cat(truth), torch.cat(hard)


def save_checkpoint(model, space, out, name):
    path = out / name
    path.mkdir(parents=True, exist_ok=True)
    if isinstance(model, SiglipHead):
        tensors = {f"vision_model.{k}": v for k, v in model.backbone.vision_model.state_dict().items()}
        tensors.update({f"classifier.{k}": v for k, v in model.classifier.state_dict().items()})
        save_file({k: v.contiguous().cpu() for k, v in tensors.items()}, path / "model.safetensors")
        cfg = model.backbone.config.to_dict()
        cfg["id2label"] = dict(enumerate(space.names))
        (path / "config.json").write_text(json.dumps(cfg, indent=1))
    else:
        model.save_pretrained(path, safe_serialization=True)


def train_finetune(args, model, space, train_rows, data_dir, val_loader, device, out, generator):
    loader = DataLoader(
        Images(data_dir, train_rows, train_transform),
        batch_size=args.batch,
        shuffle=True,
        num_workers=args.workers,
        drop_last=True,
        persistent_workers=True,
    )
    head = [p for n, p in model.named_parameters() if n.startswith("classifier")]
    body = [p for n, p in model.named_parameters() if not n.startswith("classifier")]
    optimizer = torch.optim.AdamW(
        [{"params": body, "lr": args.lr}, {"params": head, "lr": args.head_lr}], weight_decay=0.05
    )
    total = args.epochs * len(loader)
    scheduler = torch.optim.lr_scheduler.OneCycleLR(
        optimizer, max_lr=[args.lr, args.head_lr], total_steps=total, pct_start=0.1
    )
    best, history = None, []
    for epoch in range(args.epochs):
        model.train()
        running, started = 0.0, time.time()
        for step, (x, pos, hard) in enumerate(loader):
            pos, hard = pos.to(device), hard.to(device)
            w = loss_weights(args, space, pos, hard, generator)
            with torch.autocast(device, dtype=torch.bfloat16, enabled=device == "cuda"):
                logits = logits_of(model, x.to(device))
            loss = weighted_bce(logits, pos, w)
            optimizer.zero_grad()
            loss.backward()
            optimizer.step()
            scheduler.step()
            running += loss.item()
            if step % 100 == 0:
                print(f"epoch {epoch + 1} step {step}/{len(loader)} loss {loss.item():.4f}", flush=True)
        probs, truth, hard = predict(model, val_loader, device)
        r = report(probs, truth, hard, space)
        entry = {"epoch": epoch + 1, "loss": running / len(loader), "seconds": round(time.time() - started), **summary(r)}
        history.append(entry)
        print(json.dumps(entry), flush=True)
        save_checkpoint(model, space, out, "last")
        if best is None or r["macro_f1"] > best[0]["macro_f1"]:
            best = (r, probs)
            save_checkpoint(model, space, out, "best")
    return best, history


def train_linear(args, model, space, train_rows, data_dir, val_loader, device, out, generator):
    cache = out.parent / f"siglip_features_{args.feature_passes}.pt"
    if cache.exists():
        feats, truth, hard, val_feats, val_truth, val_hard = torch.load(cache)
    else:
        parts = []
        for i in range(args.feature_passes):
            tf = eval_transform if i == 0 else train_transform
            loader = DataLoader(Images(data_dir, train_rows, tf), batch_size=128, num_workers=args.workers)
            parts.append(embed(model, loader, device))
            print(f"feature pass {i + 1}/{args.feature_passes}", flush=True)
        feats, truth, hard = (torch.cat([p[k] for p in parts]) for k in range(3))
        val_feats, val_truth, val_hard = embed(model, val_loader, device)
        torch.save((feats, truth, hard, val_feats, val_truth, val_hard), cache)
    head = model.classifier
    optimizer = torch.optim.AdamW(head.parameters(), lr=args.head_lr, weight_decay=1e-4)
    steps = args.epochs * (len(feats) // args.batch)
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, steps)
    feats, truth, hard = feats.to(device), truth.to(device), hard.to(device)
    best, history = None, []
    for epoch in range(args.epochs):
        head.train()
        order = torch.randperm(len(feats), device=device)
        running = 0.0
        for i in range(0, len(order) - args.batch + 1, args.batch):
            idx = order[i : i + args.batch]
            w = loss_weights(args, space, truth[idx], hard[idx], generator)
            loss = weighted_bce(head(feats[idx]), truth[idx], w)
            optimizer.zero_grad()
            loss.backward()
            optimizer.step()
            scheduler.step()
            running += loss.item()
        head.eval()
        with torch.no_grad():
            probs = torch.sigmoid(head(val_feats.to(device))).cpu()
        r = report(probs, val_truth, val_hard, space)
        entry = {"epoch": epoch + 1, "loss": running / max(1, len(order) // args.batch), **summary(r)}
        history.append(entry)
        print(json.dumps(entry), flush=True)
        if best is None or r["macro_f1"] > best[0]["macro_f1"]:
            best = (r, probs)
            save_checkpoint(model, space, out, "best")
    return best, history


def summary(r):
    keys = ["macro_f1", "micro_f1", "macro_precision", "macro_recall", "top5_hit", "top5_recall", "predicted_per_image", "hard_negative_rate", "fitted_on_val_macro_f1"]
    return {k: r[k] for k in keys}


@torch.no_grad()
def throughput(model, device, batch, rounds=10):
    model.eval()
    x = torch.randn(batch, 3, 224, 224, device=device)
    with torch.autocast(device, dtype=torch.bfloat16, enabled=device == "cuda"):
        for _ in range(3):
            logits_of(model, x)
        if device == "cuda":
            torch.cuda.synchronize()
        started = time.time()
        for _ in range(rounds):
            logits_of(model, x)
        if device == "cuda":
            torch.cuda.synchronize()
    seconds = time.time() - started
    return {"device": device, "batch": batch, "images_per_second": round(batch * rounds / seconds, 1), "ms_per_image": round(1000 * seconds / (batch * rounds), 3)}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--model", choices=sorted(BASES), required=True)
    p.add_argument("--loss", choices=["plain", "partial"], required=True)
    p.add_argument("--data", default=str(HERE / "data" / "taxonomy_v1"))
    p.add_argument("--out", default=str(HERE / "out" / "taxonomy"))
    p.add_argument("--taxonomy", default=str(HERE / "taxonomy.json"))
    p.add_argument("--base")
    p.add_argument("--epochs", type=int)
    p.add_argument("--batch", type=int, default=64)
    p.add_argument("--lr", type=float, default=3e-5)
    p.add_argument("--head-lr", type=float, default=1e-3)
    p.add_argument("--far-samples", type=int, default=16)
    p.add_argument("--far-weight", type=float, default=0.1)
    p.add_argument("--feature-passes", type=int, default=3)
    p.add_argument("--workers", type=int, default=8)
    p.add_argument("--seed", type=int, default=0)
    args = p.parse_args()
    args.base = args.base or BASES[args.model]
    args.epochs = args.epochs or (6 if args.model == "vit" else 40)

    torch.manual_seed(args.seed)
    random.seed(args.seed)
    device = "cuda" if torch.cuda.is_available() else "cpu"
    generator = torch.Generator(device=device).manual_seed(args.seed)
    space = LabelSpace(json.loads(Path(args.taxonomy).read_text()))
    data_dir = Path(args.data)
    out = Path(args.out) / f"{args.model}_{args.loss}"
    out.mkdir(parents=True, exist_ok=True)
    train_rows, val_rows = read_rows(data_dir, space)
    print(f"train {len(train_rows)} val {len(val_rows)} classes {len(space)}", flush=True)
    val_loader = DataLoader(Images(data_dir, val_rows, eval_transform), batch_size=128, num_workers=args.workers)

    model = build_model(args, space).to(device)
    trainer = train_finetune if args.model == "vit" else train_linear
    (best, probs), history = trainer(args, model, space, train_rows, data_dir, val_loader, device, out, generator)
    torch.save({"probs": probs, "shas": [r[0] for r in val_rows], "thresholds": best["thresholds"]}, out / "val_probs.pt")
    result = {
        "model": args.model,
        "base": args.base,
        "loss": args.loss,
        "args": vars(args),
        "train": len(train_rows),
        "val": len(val_rows),
        "history": history,
        "best": best,
        "throughput": [throughput(model, device, 64), throughput(model, device, 1, rounds=50)],
    }
    (out / "report.json").write_text(json.dumps(result, indent=1))
    print(json.dumps({k: result[k] for k in ["model", "loss", "throughput"]} | summary(best)), flush=True)


if __name__ == "__main__":
    main()
