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
FIELDS = ["index", "sha256", "class", "runs", "verdict"]


def extras(probs, thresholds, positives, names):
    out = []
    for i, n in enumerate(names):
        if probs[i] >= thresholds[i] and n not in positives:
            out.append((n, float(probs[i])))
    return sorted(out, key=lambda e: -e[1])


def run_extras(run_dir, positives, names):
    saved = torch.load(run_dir / "val_probs.pt")
    thresholds = torch.tensor(saved["thresholds"])
    return {sha: extras(probs, thresholds, positives[sha], names) for sha, probs in zip(saved["shas"], saved["probs"])}


def merge(per_run):
    merged = defaultdict(dict)
    for run, by_sha in per_run.items():
        for sha, found in by_sha.items():
            merged[sha]
            for cls, _ in found:
                merged[sha].setdefault(cls, []).append(run)
    return merged


def sample(run_dirs, manifest, count, seed):
    taxonomy = json.loads((HERE / "taxonomy.json").read_text())
    names = [c["name"] for c in taxonomy["classes"]]
    positives = {r["sha256"]: set(r["positives"].split("|")) for r in manifest}
    merged = merge({d.name: run_extras(d, positives, names) for d in run_dirs})
    shas = sorted(merged)
    random.Random(seed).shuffle(shas)
    return [{"sha256": s, "qwen": sorted(positives[s]), "extra": sorted(merged[s].items())} for s in shas[:count]]


def sheet(rows, data_dir, out, tags, cols=5, cell=256):
    text_h = 120
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
        for j, (n, runs) in enumerate(r["extra"][:8]):
            mark = "".join(tags[run] for run in runs)
            draw.text((x + 4, y + cell + 14 + j * 13), f"{chr(97 + j)} {n} [{mark}]", fill="black")
    img.save(out)


def tally(rows, groups):
    table = defaultdict(lambda: [0, 0])
    for r in rows:
        if r["verdict"] not in VERDICTS:
            continue
        for run in r["runs"].split("|"):
            for key in ((run, "all"), (run, groups[r["class"]])):
                table[key][0] += r["verdict"] == "right"
                table[key][1] += 1
    return dict(table)


def read_sample(path):
    with open(path) as f:
        return list(csv.DictReader(f))


def write_sample(path, rows):
    with open(path, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=FIELDS)
        w.writeheader()
        w.writerows(rows)


def read_right_list(path):
    right = {}
    for line in Path(path).read_text().splitlines():
        if ":" not in line:
            continue
        index, classes = line.split(":", 1)
        right[index.strip()] = {c.strip() for c in classes.split(",") if c.strip()}
    return right


def apply_right_list(rows, right):
    for r in rows:
        if r["index"] in right:
            r["verdict"] = "right" if r["class"] in right[r["index"]] else "wrong"
    return rows


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("sheets")
    a.add_argument("runs", nargs="+")
    a.add_argument("--count", type=int, default=200)
    a.add_argument("--seed", type=int, default=1441)
    a.add_argument("--per-sheet", type=int, default=20)
    a.add_argument("--out", default="/tmp/r96-recall")
    b = sub.add_parser("report")
    b.add_argument("sample")
    c = sub.add_parser("import")
    c.add_argument("sample")
    c.add_argument("right_list")
    for s in (a, b, c):
        s.add_argument("--data", default=str(DATA))
    args = p.parse_args()
    data_dir = Path(args.data)
    if args.cmd == "sheets":
        with open(data_dir / "manifest.csv") as f:
            manifest = list(csv.DictReader(f))
        run_dirs = [Path(r) for r in args.runs]
        tags = {d.name: chr(65 + i) for i, d in enumerate(run_dirs)}
        rows = sample(run_dirs, manifest, args.count, args.seed)
        out = Path(args.out)
        out.mkdir(parents=True, exist_ok=True)
        flat = []
        for i, r in enumerate(rows):
            r["index"] = i
            for n, runs in r["extra"]:
                flat.append({"index": i, "sha256": r["sha256"], "class": n, "runs": "|".join(runs), "verdict": ""})
        write_sample(out / "sample.csv", flat)
        for s in range(0, len(rows), args.per_sheet):
            sheet(rows[s : s + args.per_sheet], data_dir, out / f"sheet_{s // args.per_sheet:02d}.png", tags)
        print(f"{len(rows)} images, {len(flat)} extra labels, tags {tags}, sheets in {out}")
    elif args.cmd == "import":
        rows = apply_right_list(read_sample(Path(args.sample)), read_right_list(args.right_list))
        write_sample(Path(args.sample), rows)
        print(f"{sum(r['verdict'] in VERDICTS for r in rows)} of {len(rows)} graded")
    else:
        taxonomy = json.loads((HERE / "taxonomy.json").read_text())
        groups = {c["name"]: c["group"] for c in taxonomy["classes"]}
        table = tally(read_sample(Path(args.sample)), groups)
        print("| run | group | extra labels | right | precision |\n|---|---|---|---|---|")
        for run, key in sorted(table, key=lambda k: (k[0], k[1] != "all", -table[k][1])):
            right, total = table[(run, key)]
            print(f"| {run} | {key} | {total} | {right} | {100 * right / total:.0f}% |")


if __name__ == "__main__":
    main()
