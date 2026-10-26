import argparse
import csv
import random
import re
from pathlib import Path

from PIL import Image, ImageDraw

from export_dataset import SCREENSHOT_PATTERN


def select(rows, group):
    if group == "falcon":
        return [r for r in rows if r["kind"] == "hard_negative" and not any(SCREENSHOT_PATTERN.search(l) for l in r["llm_labels"].split("|"))]
    if group == "screenshot":
        return [r for r in rows if r["kind"] == "hard_negative" and any(SCREENSHOT_PATTERN.search(l) for l in r["llm_labels"].split("|"))]
    return [r for r in rows if r["kind"] == group]


def main():
    p = argparse.ArgumentParser()
    p.add_argument("group", choices=["falcon", "screenshot", "normal", "positive"])
    p.add_argument("--data", default=str(Path(__file__).parent / "data"))
    p.add_argument("--count", type=int, default=48)
    p.add_argument("--cols", type=int, default=8)
    p.add_argument("--cell", type=int, default=200)
    p.add_argument("--seed", type=int, default=1)
    p.add_argument("--out", default="/tmp/sheet.png")
    args = p.parse_args()

    data = Path(args.data)
    rows = select(list(csv.DictReader(open(data / "manifest.csv"))), args.group)
    random.Random(args.seed).shuffle(rows)
    rows = rows[: args.count]
    n_rows = -(-len(rows) // args.cols)
    sheet = Image.new("RGB", (args.cols * args.cell, n_rows * args.cell), "white")
    draw = ImageDraw.Draw(sheet)
    for i, r in enumerate(rows):
        img = Image.open(data / "images" / f"{r['sha256']}.jpg")
        img.thumbnail((args.cell - 4, args.cell - 4))
        x, y = (i % args.cols) * args.cell, (i // args.cols) * args.cell
        sheet.paste(img, (x + 2, y + 2))
        draw.rectangle((x + 2, y + 2, x + 24, y + 14), fill="black")
        draw.text((x + 4, y + 3), str(i), fill="yellow")
        print(i, r["sha256"][:12], r["llm_labels"][:90])
    sheet.save(args.out)


if __name__ == "__main__":
    main()
