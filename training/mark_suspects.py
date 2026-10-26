import argparse
import csv
import shutil
from pathlib import Path

NEGATIVE_KINDS = {"normal", "hard_negative"}


def mark(manifest_rows, scores, threshold):
    marked = 0
    for r in manifest_rows:
        if r["kind"] in NEGATIVE_KINDS and any(
            float(scores[r["sha256"]][m]) >= threshold for m in scores[r["sha256"]] if m != "sha256"
        ):
            r["kind"] = "suspect"
            r["label"] = "nsfw"
            marked += 1
    return marked


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--data", default=str(Path(__file__).parent / "data"))
    p.add_argument("--threshold", type=float, default=0.5)
    args = p.parse_args()
    data = Path(args.data)
    rows = list(csv.DictReader(open(data / "manifest.csv")))
    scores = {r["sha256"]: r for r in csv.DictReader(open(data / "scores.csv"))}
    shutil.copy(data / "manifest.csv", data / "manifest.before_suspects.csv")
    marked = mark(rows, scores, args.threshold)
    with open(data / "manifest.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
        w.writeheader()
        w.writerows(rows)
    print(f"{marked} marked suspect")


if __name__ == "__main__":
    main()
