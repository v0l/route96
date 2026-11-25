import argparse
import csv
import os
import random
from pathlib import Path

from export_dataset import fetch_batch, kubectl, resize

HERE = Path(__file__).parent
QUERY = """
select lower(hex(u.id)), group_concat(l.label order by l.label separator '|')
from uploads u
join upload_labels l on l.file = u.id and l.model = '{model}'
where u.mime_type in ('image/jpeg', 'image/png', 'image/webp')
group by u.id
"""


def sample(rows, count, seed):
    rows = sorted(rows)
    random.Random(seed).shuffle(rows)
    return rows[:count]


def fetch_into(args, img_dir, shas):
    if not shas:
        return
    source = argparse.Namespace(**{**vars(args), "storage_dir": args.storage_dir_one})
    for sha, data in fetch_batch(source, shas):
        try:
            resize(data, args.short_side).save(img_dir / f"{sha}.jpg", quality=90)
        except Exception as e:
            print(f"bad image {sha}: {e}")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--out", default=str(HERE / "data" / "label_quality"))
    p.add_argument("--namespace", default="nostr-download")
    p.add_argument("--app-deploy", default="app")
    p.add_argument("--db-deploy", default="mysql")
    p.add_argument("--db-password", default=os.environ.get("DB_PASSWORD"), required=not os.environ.get("DB_PASSWORD"))
    p.add_argument("--model", default="qwen3.5")
    p.add_argument("--storage-dir", action="append", default=None)
    p.add_argument("--short-side", type=int, default=256)
    p.add_argument("--count", type=int, default=300)
    p.add_argument("--seed", type=int, default=1407)
    args = p.parse_args()

    out = Path(args.out)
    img_dir = out / "images"
    img_dir.mkdir(parents=True, exist_ok=True)
    raw = kubectl(
        args.namespace, args.db_deploy, "mariadb", "-uroot", f"-p{args.db_password}", "route96", "-N", "-B",
        "-e", QUERY.format(model=args.model),
    )
    rows = [tuple(line.split("\t")) for line in raw.decode().splitlines() if "\t" in line]
    picked = sample(rows, args.count + args.count // 5, args.seed)
    todo = [r for r in picked if not (img_dir / f"{r[0]}.jpg").exists()]
    for storage in args.storage_dir or ["/data", "/cold"]:
        args.storage_dir_one = storage
        fetch_into(args, img_dir, [r[0] for r in todo if not (img_dir / f"{r[0]}.jpg").exists()])
    kept = [r for r in picked if (img_dir / f"{r[0]}.jpg").exists()][: args.count]
    with open(out / "manifest.csv", "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["sha256", "llm_labels"])
        w.writerows(kept)
    print(f"{len(rows)} labelled images, {len(kept)} sampled into {out}")


if __name__ == "__main__":
    main()
