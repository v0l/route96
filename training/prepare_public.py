import argparse
import csv
import io
import random
from pathlib import Path

import pyarrow.parquet as pq
from PIL import Image

from export_dataset import resize


def save(data, path, short_side):
    resize(data, short_side).save(path, quality=92)


def xl(root, out, count, rng, short_side):
    base = root / "x/xl/nsfw_detection_xl"
    rows = []
    for folder, label in (("sfw", "normal"), ("nsfw", "nsfw")):
        files = sorted((base / folder).glob("*.jpg"))
        rng.shuffle(files)
        for f in files[:count]:
            sha = f"xl_{f.stem}"
            try:
                save(f.read_bytes(), out / "images" / f"{sha}.jpg", short_side)
            except Exception:
                continue
            rows.append((sha, label, "public_xl", "image/jpeg", folder))
    return rows


def cashbox(root, out, count, rng, short_side):
    names = {0: "L1", 1: "L2", 2: "L3", 3: "L4"}
    pools = {"nsfw": [], "L1": []}
    for shard in sorted((root / "cashbox/data").glob("*.parquet")):
        table = pq.read_table(shard)
        for img, label in zip(table["image"].to_pylist(), table["label"].to_pylist()):
            if label in (2, 3):
                pools["nsfw"].append((img["bytes"], names[label]))
            elif label == 0:
                pools["L1"].append((img["bytes"], "L1"))
    rows = []
    for pool, kind in (("nsfw", "public_cashbox_explicit"), ("L1", "public_cashbox_suggestive")):
        rng.shuffle(pools[pool])
        for i, (data, level) in enumerate(pools[pool][:count]):
            sha = f"cashbox_{level}_{i}"
            try:
                save(data, out / "images" / f"{sha}.jpg", short_side)
            except Exception:
                continue
            rows.append((sha, "nsfw" if pool == "nsfw" else "normal", kind, "image/jpeg", level))
    return rows


def darky(root, out, count, rng, short_side):
    files = sorted((root / "x/darky/nsfw").iterdir())
    rng.shuffle(files)
    rows = []
    for f in files[:count]:
        sha = f"darky_{f.stem}"
        try:
            save(f.read_bytes(), out / "images" / f"{sha}.jpg", short_side)
        except Exception:
            continue
        rows.append((sha, "nsfw", "public_darky", "image/jpeg", "nsfw"))
    return rows


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--root", default="public")
    p.add_argument("--out", default="public/eval")
    p.add_argument("--count", type=int, default=1500)
    p.add_argument("--short-side", type=int, default=288)
    args = p.parse_args()
    root, out = Path(args.root), Path(args.out)
    (out / "images").mkdir(parents=True, exist_ok=True)
    rng = random.Random(0)
    rows = []
    for fn in (xl, cashbox, darky):
        rows += fn(root, out, args.count, rng, args.short_side)
    with open(out / "manifest.csv", "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["sha256", "label", "kind", "mime", "llm_labels"])
        w.writerows(rows)
    print(len(rows))


if __name__ == "__main__":
    main()
