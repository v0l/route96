import argparse
import csv
import json
import random
from collections import Counter, defaultdict
from pathlib import Path

from export_dataset import fetch_batch, kubectl, resize
from taxonomy import DECISIONS, OUT, class_lookup, labels_to_classes, read_hard_negatives

HERE = Path(__file__).parent
QUERY = (
    "select lower(hex(u.id)), group_concat(l.label separator '|') "
    "from uploads u join upload_labels l on l.file = u.id and l.model = '{model}' "
    "where u.mime_type in ('image/jpeg', 'image/png', 'image/webp') group by u.id"
)
FIELDS = ["sha256", "split", "positives", "hard_negatives", "llm_labels"]


def split_shas(shas, val_fraction, seed):
    rows = sorted(shas)
    random.Random(seed).shuffle(rows)
    cut = int(len(rows) * val_fraction)
    return set(rows[:cut])


def select(image_classes, min_per_class, cap, seed):
    by_class = defaultdict(list)
    for sha in sorted(image_classes):
        for c in image_classes[sha]:
            by_class[c].append(sha)
    rng = random.Random(seed)
    for shas in by_class.values():
        rng.shuffle(shas)
    picked = set()
    counts = Counter()
    for c in sorted(by_class, key=lambda c: (len(by_class[c]), c)):
        for sha in by_class[c]:
            if counts[c] >= cap:
                break
            if sha in picked:
                continue
            picked.add(sha)
            counts.update(image_classes[sha])
    short = [c for c in by_class if counts[c] < min(min_per_class, len(by_class[c]))]
    if short:
        raise ValueError(f"classes under the floor: {short}")
    return picked


def hard_negatives_for(positives, pairs):
    pos = set(positives)
    return sorted({neg for label, neg in pairs if label in pos and neg not in pos})


def check_pairs(pairs, class_names):
    unknown = sorted({c for pair in pairs for c in pair if c not in class_names})
    if unknown:
        raise ValueError(f"hard negative decisions name classes outside the taxonomy: {unknown}")


def manifest_rows(labelled, taxonomy, pairs, min_per_class, cap, val_fraction, seed):
    lookup = class_lookup(taxonomy)
    check_pairs(pairs, {c["name"] for c in taxonomy["classes"]})
    image_classes = {}
    for sha, labels in labelled.items():
        classes = labels_to_classes(labels, lookup)
        if classes:
            image_classes[sha] = classes
    picked = select(image_classes, min_per_class, cap, seed)
    val = split_shas(picked, val_fraction, seed)
    rows = []
    for sha in sorted(picked):
        rows.append(
            {
                "sha256": sha,
                "split": "val" if sha in val else "train",
                "positives": "|".join(image_classes[sha]),
                "hard_negatives": "|".join(hard_negatives_for(image_classes[sha], pairs)),
                "llm_labels": "|".join(sorted(labelled[sha])),
            }
        )
    return rows


def read_manifest(path):
    with open(path) as f:
        return list(csv.DictReader(f))


def write_manifest(path, rows):
    with open(path, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=FIELDS)
        w.writeheader()
        w.writerows(rows)


def query_labels(args):
    sql = QUERY.format(model=args.model)
    raw = kubectl(
        args.namespace,
        args.db_deploy,
        "sh",
        "-c",
        'exec mariadb -uroot -p"$MARIADB_ROOT_PASSWORD" route96 -N -B -e "$0"',
        sql,
    )
    labelled = {}
    for line in raw.decode().splitlines():
        if "\t" not in line:
            continue
        sha, labels = line.split("\t", 1)
        labelled[sha] = {l for l in labels.split("|") if l}
    return labelled


def fetch_images(args, img_dir, shas):
    for storage in args.storage_dir:
        todo = [s for s in shas if not (img_dir / f"{s}.jpg").exists()]
        source = argparse.Namespace(**{**vars(args), "storage_dir": storage})
        for i in range(0, len(todo), args.batch):
            for sha, data in fetch_batch(source, todo[i : i + args.batch]):
                try:
                    resize(data, args.short_side).save(img_dir / f"{sha}.jpg", quality=90)
                except Exception as e:
                    print(f"bad image {sha}: {e}")
            print(f"{storage} {min(i + args.batch, len(todo))}/{len(todo)}", flush=True)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--out", default=str(HERE / "data" / "taxonomy_v1"))
    p.add_argument("--taxonomy", default=str(OUT))
    p.add_argument("--decisions", default=str(DECISIONS))
    p.add_argument("--namespace", default="nostr-download")
    p.add_argument("--app-deploy", default="app")
    p.add_argument("--db-deploy", default="mysql")
    p.add_argument("--model", default="qwen3.5")
    p.add_argument("--storage-dir", action="append", default=None)
    p.add_argument("--short-side", type=int, default=256)
    p.add_argument("--batch", type=int, default=200)
    p.add_argument("--min-per-class", type=int, default=200)
    p.add_argument("--cap", type=int, default=2000)
    p.add_argument("--val-fraction", type=float, default=0.15)
    p.add_argument("--seed", type=int, default=0)
    args = p.parse_args()
    args.storage_dir = args.storage_dir or ["/data", "/cold"]

    out = Path(args.out)
    img_dir = out / "images"
    img_dir.mkdir(parents=True, exist_ok=True)
    taxonomy = json.loads(Path(args.taxonomy).read_text())
    pairs = read_hard_negatives(Path(args.decisions))
    labelled = query_labels(args)
    rows = manifest_rows(labelled, taxonomy, pairs, args.min_per_class, args.cap, args.val_fraction, args.seed)
    print(f"{len(labelled)} labelled images, {len(rows)} selected", flush=True)
    fetch_images(args, img_dir, [r["sha256"] for r in rows])
    kept = [r for r in rows if (img_dir / f"{r['sha256']}.jpg").exists()]
    write_manifest(out / "manifest.csv", kept)
    counts = Counter(c for r in kept for c in r["positives"].split("|"))
    print(f"{len(kept)} images written, {sum(r['split'] == 'val' for r in kept)} val")
    print(f"smallest class {min(counts.values())}, largest {max(counts.values())}")


if __name__ == "__main__":
    main()
