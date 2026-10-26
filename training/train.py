import argparse
import csv
import json
import random
from pathlib import Path

import torch
from PIL import Image, ImageEnhance
from torch.utils.data import DataLoader, Dataset, WeightedRandomSampler
from transformers import ViTForImageClassification

LABELS = ["normal", "nsfw"]


def read_manifest(data_dir):
    with open(data_dir / "manifest.csv") as f:
        return [
            (r["sha256"], LABELS.index(r["label"]), r["kind"]) for r in csv.DictReader(f)
        ]


def split(rows, val_fraction, seed):
    rng = random.Random(seed)
    train, val = [], []
    for kind in sorted({r[2] for r in rows}):
        group = sorted(r for r in rows if r[2] == kind)
        rng.shuffle(group)
        cut = int(len(group) * val_fraction)
        val += group[:cut]
        train += group[cut:]
    return train, val


class Images(Dataset):
    def __init__(self, data_dir, rows, tf):
        self.dir = data_dir / "images"
        self.rows = rows
        self.tf = tf

    def __len__(self):
        return len(self.rows)

    def __getitem__(self, i):
        sha, label, _ = self.rows[i]
        return self.tf(Image.open(self.dir / f"{sha}.jpg").convert("RGB")), label


def to_tensor(img):
    arr = torch.frombuffer(bytearray(img.tobytes()), dtype=torch.uint8)
    arr = arr.view(img.size[1], img.size[0], 3).permute(2, 0, 1).float() / 255.0
    return (arr - 0.5) / 0.5


def train_transform(img):
    w, h = img.size
    scale = random.uniform(0.6, 1.0)
    cw, ch = max(1, int(w * scale)), max(1, int(h * scale))
    x, y = random.randint(0, w - cw), random.randint(0, h - ch)
    img = img.crop((x, y, x + cw, y + ch)).resize((224, 224), Image.BILINEAR)
    if random.random() < 0.5:
        img = img.transpose(Image.FLIP_LEFT_RIGHT)
    img = ImageEnhance.Brightness(img).enhance(random.uniform(0.8, 1.2))
    img = ImageEnhance.Contrast(img).enhance(random.uniform(0.8, 1.2))
    return to_tensor(img)


def eval_transform(img):
    return to_tensor(img.resize((224, 224), Image.BILINEAR))


@torch.no_grad()
def probabilities(model, loader, device):
    model.eval()
    probs, labels = [], []
    for x, y in loader:
        logits = model(pixel_values=x.to(device)).logits
        probs.append(torch.softmax(logits.float(), -1)[:, 1].cpu())
        labels.append(y)
    return torch.cat(probs), torch.cat(labels)


def hard_negative_fpr(probs, kinds, threshold=0.5):
    mask = torch.tensor([k == "hard_negative" for k in kinds])
    if not mask.any():
        return None
    return round((probs[mask] >= threshold).float().mean().item(), 4)


def metrics(probs, labels, threshold=0.5):
    pred = probs >= threshold
    pos = labels == 1
    tp = (pred & pos).sum().item()
    fp = (pred & ~pos).sum().item()
    fn = (~pred & pos).sum().item()
    precision = tp / max(tp + fp, 1)
    recall = tp / max(tp + fn, 1)
    f1 = 2 * precision * recall / max(precision + recall, 1e-9)
    return {"precision": precision, "recall": recall, "f1": f1, "tp": tp, "fp": fp, "fn": fn}


def best_threshold(probs, labels):
    candidates = [i / 100 for i in range(5, 96, 5)]
    return max(candidates, key=lambda t: metrics(probs, labels, t)["f1"])


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--data", default=str(Path(__file__).parent / "data"))
    p.add_argument("--out", default=str(Path(__file__).parent / "out"))
    p.add_argument("--base", default="Falconsai/nsfw_image_detection")
    p.add_argument("--epochs", type=int, default=5)
    p.add_argument("--batch", type=int, default=32)
    p.add_argument("--lr", type=float, default=2e-5)
    p.add_argument("--val-fraction", type=float, default=0.15)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--workers", type=int, default=8)
    args = p.parse_args()

    torch.manual_seed(args.seed)
    device = "cuda" if torch.cuda.is_available() else "cpu"
    data_dir, out = Path(args.data), Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    model = ViTForImageClassification.from_pretrained(args.base).to(device)
    id2label = {int(k): v.lower() for k, v in model.config.id2label.items()}
    assert [id2label[i] for i in range(len(LABELS))] == LABELS, id2label

    train_tf, eval_tf = train_transform, eval_transform
    train_rows, val_rows = split(read_manifest(data_dir), args.val_fraction, args.seed)
    print(f"train {len(train_rows)} val {len(val_rows)}")

    counts = [sum(1 for r in train_rows if r[1] == c) for c in range(len(LABELS))]
    weights = [(2.0 if r[2] == "hard_negative" else 1.0) / counts[r[1]] for r in train_rows]
    sampler = WeightedRandomSampler(weights, num_samples=len(train_rows), replacement=True)
    train_loader = DataLoader(
        Images(data_dir, train_rows, train_tf),
        batch_size=args.batch,
        sampler=sampler,
        num_workers=args.workers,
        drop_last=True,
    )
    val_loader = DataLoader(
        Images(data_dir, val_rows, eval_tf), batch_size=64, num_workers=args.workers
    )

    probs, labels = probabilities(model, val_loader, device)
    val_kinds = [r[2] for r in val_rows]
    baseline = {
        "threshold_0.5": metrics(probs, labels),
        "best_threshold": best_threshold(probs, labels),
        "hard_negative_fpr": hard_negative_fpr(probs, val_kinds),
    }
    print("baseline", baseline)

    optimizer = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=0.01)
    total = args.epochs * len(train_loader)
    scheduler = torch.optim.lr_scheduler.OneCycleLR(
        optimizer, max_lr=args.lr, total_steps=total, pct_start=0.1
    )
    best_f1, best = -1.0, None
    for epoch in range(args.epochs):
        model.train()
        running = 0.0
        for step, (x, y) in enumerate(train_loader):
            with torch.autocast(device, dtype=torch.bfloat16, enabled=device == "cuda"):
                loss = torch.nn.functional.cross_entropy(
                    model(pixel_values=x.to(device)).logits.float(), y.to(device)
                )
            optimizer.zero_grad()
            loss.backward()
            optimizer.step()
            scheduler.step()
            running += loss.item()
        probs, labels = probabilities(model, val_loader, device)
        result = metrics(probs, labels)
        result["hard_negative_fpr"] = hard_negative_fpr(probs, val_kinds)
        print(f"epoch {epoch + 1} loss {running / len(train_loader):.4f} val {result}")
        if result["f1"] > best_f1:
            best_f1 = result["f1"]
            best = {"epoch": epoch + 1, "val": result, "best_threshold": best_threshold(probs, labels)}
            model.save_pretrained(out / "model", safe_serialization=True)

    report = {"baseline": baseline, "best": best, "train": len(train_rows), "val": len(val_rows)}
    (out / "report.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
