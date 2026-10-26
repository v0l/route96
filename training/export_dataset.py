import argparse
import csv
import io
import os
import re
import subprocess
import tarfile
from pathlib import Path

from PIL import Image, ImageOps

NSFW_PATTERN = re.compile(r"nsfw|nipple|brassiere|nude|topless|breast", re.I)
BENIGN_PATTERN = re.compile(
    r"clamp|pump|feeding|cancer|piercing|jewelry|pasties|chicken|turkey|stroke|plate", re.I
)
IMAGE_MIMES = ("image/jpeg", "image/png", "image/webp")

QUERY = """
select hex(u.id), u.mime_type, coalesce(group_concat(l.label separator '|'), '')
from uploads u
join upload_labeled_by b on b.file = u.id and b.model = '{model}'
left join upload_labels l on l.file = u.id and l.model = '{model}'
where u.mime_type in ('image/jpeg', 'image/png', 'image/webp')
group by u.id, u.mime_type
"""


def kubectl(namespace, deploy, *args, stdin=None):
    result = subprocess.run(
        ["kubectl", "-n", namespace, "exec", "-i", f"deploy/{deploy}", "--", *args],
        input=stdin,
        capture_output=True,
    )
    if result.returncode != 0 and not (args[0] == "tar" and result.stdout):
        raise RuntimeError(result.stderr.decode())
    return result.stdout


def classify(labels):
    hits = [l for l in labels if NSFW_PATTERN.search(l)]
    if not hits:
        return "normal"
    if all(BENIGN_PATTERN.search(l) for l in hits):
        return None
    return "nsfw"


def fetch_rows(args):
    out = kubectl(
        args.namespace,
        args.db_deploy,
        "mariadb",
        "-uroot",
        f"-p{args.db_password}",
        "route96",
        "-N",
        "-B",
        "-e",
        QUERY.format(model=args.model),
    )
    for line in out.decode().splitlines():
        sha, mime, labels = line.split("\t")
        yield sha.lower(), mime, [l for l in labels.split("|") if l]


def blob_path(sha):
    return f"{sha[0:2]}/{sha[2:4]}/{sha}"


def resize(data, short_side):
    img = ImageOps.exif_transpose(Image.open(io.BytesIO(data)))
    img = img.convert("RGB")
    w, h = img.size
    scale = short_side / min(w, h)
    if scale < 1:
        img = img.resize((max(1, round(w * scale)), max(1, round(h * scale))), Image.LANCZOS)
    return img


def fetch_batch(args, shas):
    names = "\n".join(blob_path(s) for s in shas).encode()
    raw = kubectl(
        args.namespace,
        args.app_deploy,
        "tar",
        "-C",
        args.storage_dir,
        "-cf",
        "-",
        "-T",
        "-",
        stdin=names,
    )
    try:
        with tarfile.open(fileobj=io.BytesIO(raw)) as tar:
            for member in tar:
                if member.isfile():
                    yield member.name.rsplit("/", 1)[-1], tar.extractfile(member).read()
    except tarfile.ReadError as e:
        print(f"truncated batch: {e}")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--out", default=str(Path(__file__).parent / "data"))
    p.add_argument("--namespace", default="nostr-download")
    p.add_argument("--app-deploy", default="app")
    p.add_argument("--db-deploy", default="mysql")
    p.add_argument("--db-password", default=os.environ.get("DB_PASSWORD"), required=not os.environ.get("DB_PASSWORD"))
    p.add_argument("--model", default="qwen3.5")
    p.add_argument("--storage-dir", default="/data")
    p.add_argument("--short-side", type=int, default=288)
    p.add_argument("--batch", type=int, default=200)
    p.add_argument("--max-normal", type=int, default=0)
    args = p.parse_args()

    out = Path(args.out)
    img_dir = out / "images"
    img_dir.mkdir(parents=True, exist_ok=True)

    rows = []
    skipped = 0
    for sha, mime, labels in fetch_rows(args):
        label = classify(labels)
        if label is None:
            skipped += 1
            continue
        rows.append((sha, label, mime, "|".join(labels)))
    if args.max_normal:
        normals = [r for r in rows if r[1] == "normal"][: args.max_normal]
        rows = [r for r in rows if r[1] == "nsfw"] + normals
    print(f"{len(rows)} files, {skipped} ambiguous skipped")

    todo = [r for r in rows if not (img_dir / f"{r[0]}.jpg").exists()]
    done_missing = set()
    for i in range(0, len(todo), args.batch):
        batch = todo[i : i + args.batch]
        got = set()
        for sha, data in fetch_batch(args, [r[0] for r in batch]):
            try:
                resize(data, args.short_side).save(img_dir / f"{sha}.jpg", quality=92)
                got.add(sha)
            except Exception as e:
                print(f"bad image {sha}: {e}")
        done_missing |= {r[0] for r in batch} - got
        print(f"{min(i + args.batch, len(todo))}/{len(todo)}")

    with open(out / "manifest.csv", "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["sha256", "label", "mime", "llm_labels"])
        for r in rows:
            if r[0] not in done_missing and (img_dir / f"{r[0]}.jpg").exists():
                w.writerow(r)


if __name__ == "__main__":
    main()
