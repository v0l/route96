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


def label_sheet(data, rows, cols, cell, out):
    text_h = 64
    n_rows = -(-len(rows) // cols)
    sheet = Image.new("RGB", (cols * cell, n_rows * (cell + text_h)), "white")
    draw = ImageDraw.Draw(sheet)
    for i, r in enumerate(rows):
        img = Image.open(data / "images" / f"{r['sha256']}.jpg")
        img.thumbnail((cell - 4, cell - 4))
        x, y = (i % cols) * cell, (i // cols) * (cell + text_h)
        sheet.paste(img, (x + 2, y + 2))
        draw.rectangle((x + 2, y + 2, x + 30, y + 14), fill="black")
        draw.text((x + 4, y + 3), str(r["index"]), fill="yellow")
        for j, label in enumerate(r["llm_labels"].split("|")[:5]):
            draw.text((x + 4, y + cell + j * 12), f"{j + 1} {label[:34]}", fill="black")
    sheet.save(out)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("group", choices=["falcon", "screenshot", "normal", "positive", "labels"])
    p.add_argument("--data", default=str(Path(__file__).parent / "data"))
    p.add_argument("--count", type=int, default=48)
    p.add_argument("--start", type=int, default=0)
    p.add_argument("--cols", type=int, default=8)
    p.add_argument("--cell", type=int, default=200)
    p.add_argument("--seed", type=int, default=1)
    p.add_argument("--out", default="/tmp/sheet.png")
    args = p.parse_args()

    data = Path(args.data)
    rows = list(csv.DictReader(open(data / "manifest.csv")))
    if args.group == "labels":
        for i, r in enumerate(rows):
            r["index"] = i
        rows = rows[args.start : args.start + args.count]
        label_sheet(data, rows, args.cols, args.cell, args.out)
        for r in rows:
            print(r["index"], r["sha256"][:12], r["llm_labels"][:120])
        return
    rows = select(rows, args.group)
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
