import argparse
import csv
import json
import random
from collections import defaultdict
from pathlib import Path

import torch
from PIL import Image, ImageDraw

HERE = Path(__file__).parent
DATA = HERE / "data" / "taxonomy_v1"
VERDICTS = ("right", "wrong")


def extras(probs, thresholds, positives, names):
    out = []
    for i, n in enumerate(names):
        if probs[i] >= thresholds[i] and n not in positives:
            out.append((n, float(probs[i])))
    return sorted(out, key=lambda e: -e[1])


def sample(run_dir, manifest, count, seed):
    saved = torch.load(run_dir / "val_probs.pt")
    taxonomy = json.loads((HERE / "taxonomy.json").read_text())
    names = [c["name"] for c in taxonomy["classes"]]
    positives = {r["sha256"]: set(r["positives"].split("|")) for r in manifest}
    thresholds = torch.tensor(saved["thresholds"])
    rows = []
    for sha, probs in zip(saved["shas"], saved["probs"]):
        rows.append({"sha256": sha, "qwen": sorted(positives[sha]), "extra": extras(probs, thresholds, positives[sha], names)})
    rows.sort(key=lambda r: r["sha256"])
    random.Random(seed).shuffle(rows)
    return rows[:count]


def sheet(rows, data_dir, out, cols=5, cell=256):
    text_h = 110
    n_rows = -(-len(rows) // cols)
    img = Image.new("RGB", (cols * cell, n_rows * (cell + text_h)), "white")
    draw = ImageDraw.Draw(img)
    for i, r in enumerate(rows):
        pic = Image.open(data_dir / "images" / f"{r['sha256']}.jpg")
        pic.thumbnail((cell - 4, cell - 4))
        x, y = (i % cols) * cell, (i // cols) * (cell + text_h)
        img.paste(pic, (x + 2, y + 2))
        draw.rectangle((x + 2, y + 2, x + 34, y + 14), fill="black")
        draw.text((x + 4, y + 3), str(r["index"]), fill="yellow")
        draw.text((x + 4, y + cell), ("q: " + ", ".join(r["qwen"]))[:44], fill="gray")
        for j, (n, p) in enumerate(r["extra"][:7]):
            draw.text((x + 4, y + cell + 14 + j * 13), f"{chr(97 + j)} {n} {p:.2f}", fill="black")
    img.save(out)


def tally(verdicts, groups):
    table = defaultdict(lambda: [0, 0])
    for (_, cls), verdict in verdicts.items():
        if verdict not in VERDICTS:
            continue
        for key in ("all", groups[cls]):
            table[key][0] += verdict == "right"
            table[key][1] += 1
    return dict(table)


def read_verdicts(path):
    with open(path) as f:
        return {(r["sha256"], r["class"]): r["verdict"] for r in csv.DictReader(f)}


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("sheets")
    a.add_argument("run")
    a.add_argument("--count", type=int, default=200)
    a.add_argument("--seed", type=int, default=1441)
    a.add_argument("--per-sheet", type=int, default=20)
    a.add_argument("--out", default="/tmp/r96-recall")
    b = sub.add_parser("report")
    b.add_argument("verdicts")
    for s in (a, b):
        s.add_argument("--data", default=str(DATA))
    args = p.parse_args()
    data_dir = Path(args.data)
    if args.cmd == "sheets":
        with open(data_dir / "manifest.csv") as f:
            manifest = list(csv.DictReader(f))
        rows = sample(Path(args.run), manifest, args.count, args.seed)
        out = Path(args.out)
        out.mkdir(parents=True, exist_ok=True)
        with open(out / "sample.csv", "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["index", "sha256", "class", "prob", "verdict"])
            for i, r in enumerate(rows):
                r["index"] = i
                for n, prob in r["extra"]:
                    w.writerow([i, r["sha256"], n, f"{prob:.3f}", ""])
        for s in range(0, len(rows), args.per_sheet):
            sheet(rows[s : s + args.per_sheet], data_dir, out / f"sheet_{s // args.per_sheet:02d}.png")
        print(f"{len(rows)} images, {sum(len(r['extra']) for r in rows)} extra labels, sheets in {out}")
    else:
        taxonomy = json.loads((HERE / "taxonomy.json").read_text())
        groups = {c["name"]: c["group"] for c in taxonomy["classes"]}
        table = tally(read_verdicts(Path(args.verdicts)), groups)
        print("| group | extra labels | right | precision |\n|---|---|---|---|")
        for key in sorted(table, key=lambda k: (k != "all", -table[k][1])):
            right, total = table[key]
            print(f"| {key} | {total} | {right} | {100 * right / total:.0f}% |")


if __name__ == "__main__":
    main()
