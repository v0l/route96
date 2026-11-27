import argparse
import csv
import json
from pathlib import Path

from workdir import OUT

HERE = Path(__file__).parent
RUNS = ["siglip_plain", "siglip_partial", "vit_plain", "vit_partial"]
PER_CLASS = ["class", "support", "predicted", "precision", "recall", "f1", "threshold"]


def load(out, run):
    return json.loads((out / run / "report.json").read_text())


def write_per_class(out, reports):
    path = out / "per_class.csv"
    with open(path, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["run", *PER_CLASS])
        for run, r in reports.items():
            for row in r["best"]["per_class"]:
                w.writerow([run, *(row[k] for k in PER_CLASS)])
    return path


def summary_table(reports):
    lines = [
        "| run | macro F1 | micro F1 | macro precision | macro recall | top 5 hit | top 5 recall | labels per image | hard negative rate | GPU images/s (batch 64) | GPU ms (batch 1) |",
        "|---|---|---|---|---|---|---|---|---|---|---|",
    ]
    for run, r in reports.items():
        b = r["best"]
        big, one = r["throughput"]
        lines.append(
            f"| {run} | {b['macro_f1']:.3f} | {b['micro_f1']:.3f} | {b['macro_precision']:.3f} | {b['macro_recall']:.3f} "
            f"| {b['top5_hit']:.3f} | {b['top5_recall']:.3f} | {b['predicted_per_image']:.2f} | {b['hard_negative_rate']:.3f} "
            f"| {big['images_per_second']:.0f} | {one['ms_per_image']:.2f} |"
        )
    return "\n".join(lines)


def weakest_table(report, count, other=None):
    rows = sorted(report["best"]["per_class"], key=lambda r: (r["f1"], r["class"]))[:count]
    other_f1 = {r["class"]: r["f1"] for r in other["best"]["per_class"]} if other else {}
    lines = ["| class | val support | precision | recall | F1 | F1 with partial loss |", "|---|---|---|---|---|---|"]
    for r in rows:
        lines.append(
            f"| {r['class']} | {r['support']} | {r['precision']:.2f} | {r['recall']:.2f} | {r['f1']:.2f} | {other_f1.get(r['class'], float('nan')):.2f} |"
        )
    return "\n".join(lines)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--out", default=str(OUT))
    p.add_argument("--weakest", type=int, default=20)
    args = p.parse_args()
    out = Path(args.out)
    reports = {run: load(out, run) for run in RUNS if (out / run / "report.json").exists()}
    print(f"per class csv: {write_per_class(out, reports)}\n")
    print(summary_table(reports))
    print()
    print(weakest_table(reports["siglip_plain"], args.weakest, reports.get("siglip_partial")))


if __name__ == "__main__":
    main()
