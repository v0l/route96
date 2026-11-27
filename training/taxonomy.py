import argparse
import csv
import json
import re
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np

HERE = Path(__file__).parent
DATA = HERE / "data" / "taxonomy"
MERGES = HERE / "taxonomy_merges.csv"
DECISIONS = HERE / "taxonomy_decisions.csv"
OUT = HERE / "taxonomy.json"
HARD_NEGATIVE = "hard negative"
IMAGE_MIMES = ("image/jpeg", "image/png", "image/webp", "image/gif")
EMBED_MODEL = "sentence-transformers/all-MiniLM-L6-v2"
VERSION = "taxonomy-v2"
MIN_IMAGES = 50
CANDIDATE_MIN = 30

GROUPS = [
    "people",
    "pose and framing",
    "body and clothing",
    "text and documents",
    "screens and apps",
    "art and abstract",
    "visual style",
    "colour",
    "animals",
    "places and nature",
    "objects",
    "vehicles",
    "food and drink",
    "bitcoin and crypto",
    "memes and media",
    "symbols and logos",
    "activities and sport",
    "seasons and events",
]
GROUP_PROMPTS = {
    "people": "people, a person, a face, a crowd",
    "pose and framing": "sitting, lying down, kneeling, a back view, a close up, a selfie, a portrait",
    "body and clothing": "clothes, garments, hair, body parts, accessories",
    "text and documents": "text, a document, a letter, a certificate, a chart",
    "screens and apps": "a screenshot of an app, a website or a user interface",
    "art and abstract": "abstract art, a drawing, a painting, a fractal, a pattern",
    "visual style": "a visual style, vintage, neon, a silhouette, a texture",
    "colour": "a colour, red, blue, black, white, gold, black and white",
    "animals": "an animal, a pet, a bird",
    "places and nature": "a place, a landscape, a room, nature, weather",
    "objects": "an object, furniture, a tool, a device",
    "vehicles": "a car, a vehicle, a plane, a boat",
    "food and drink": "food, a meal, a drink",
    "bitcoin and crypto": "bitcoin, cryptocurrency, trading, blockchain, money",
    "memes and media": "a meme, a comic, a movie, a video game, a news story",
    "symbols and logos": "a logo, a symbol, a flag, a sign",
    "activities and sport": "sport, exercise, swimming, hiking",
    "seasons and events": "christmas, summer, a holiday, a concert, a protest",
}

EXCLUDED_NSFW = re.compile(
    r"\b(nsfw|nude|nudity|naked|topless|shirtless|nipples?|areolae?|breasts?|boobs?|genitals?|genitalia"
    r"|penis|vagina|pussy|anal|sex|sexual\w*|porn\w*|erotic\w*|bondage|bdsm|fetish|cum|explicit\w*"
    r"|lingerie|underwear|bra|brassiere|panty|panties|thong|cleavage|buttocks?|butt|ass|pubic\w*|sexy"
    r"|vulva|labia|adult content|masturbat\w*|orgasm|seductive|provocative|suggestive|sensual|lewd|hentai|ahegao)\b"
)
EXCLUDED_AGE = re.compile(
    r"\b(child|children|kid|kids|baby|babies|infant|toddler|teen|teenager|boy|girl|young|elderly|old man"
    r"|old woman|adult|minor|schoolgirl|loli)\b"
)

KEEP_PLURAL = {
    "glasses", "sunglasses", "news", "christmas", "mathematics", "physics", "politics", "graphics",
    "jeans", "shorts", "pants", "trousers", "series", "species", "canvas", "bus", "headphones",
    "earphones", "people", "clothes", "leggings", "pantyhose", "tights", "economics", "statistics",
    "electronics", "analytics", "athletics", "gymnastics", "cosmetics", "sports", "abs", "dreadlocks",
    "goggles", "binoculars", "scissors", "overalls", "pajamas", "lips", "stairs", "mountains", "woods",
    "outdoors", "indoors", "texas", "paris", "glass", "fishnet stockings", "stockings", "socks",
    "abdomen", "data", "media",
}
SYNONYMS = {
    "outdoors": "outdoor",
    "indoors": "indoor",
    "smiling": "smile",
    "sofa": "couch",
    "ui": "user interface",
    "app interface": "user interface",
    "dark theme": "dark mode",
    "mobile phone": "smartphone",
    "cell phone": "smartphone",
    "mandelbrot": "mandelbrot set",
    "julia": "julia set",
    "abstract art": "abstract",
    "crypto": "cryptocurrency",
    "btc": "bitcoin",
    "photograph": "photo",
    "doggy": "dog",
    "kitten": "cat",
    "puppy": "dog",
    "tshirt": "t-shirt",
    "t shirt": "t-shirt",
    "tee shirt": "t-shirt",
    "web page": "webpage",
    "web site": "website",
    "grey": "gray",
    "colour": "color",
    "colourful": "colorful",
    "black & white": "black and white",
    "monochrome": "black and white",
}

_inflect = None
IRREGULAR = {"men": "man", "women": "woman", "abdomen": "abdomen"}


def singular(word):
    global _inflect
    if word in IRREGULAR:
        return IRREGULAR[word]
    if _inflect is None:
        import inflect

        _inflect = inflect.engine()
    if word in KEEP_PLURAL or len(word) <= 3 or word.endswith(("ss", "us", "is", "ics", "men")):
        return word
    s = _inflect.singular_noun(word)
    return s if s else word


def normalise(label):
    s = label.strip().lower().replace("_", " ").replace("\u2019", "'")
    s = re.sub(r"\s+", " ", s).strip(" .,;:!?\"'()[]")
    if not s:
        return ""
    s = SYNONYMS.get(s, s)
    if s in KEEP_PLURAL:
        return SYNONYMS.get(s, s)
    words = s.split(" ")
    words[-1] = singular(words[-1])
    s = " ".join(words)
    return SYNONYMS.get(s, s)


def excluded_reason(label):
    if EXCLUDED_NSFW.search(label):
        return "nsfw"
    if EXCLUDED_AGE.search(label):
        return "age"
    return None


def read_images(path, mimes=IMAGE_MIMES):
    images = defaultdict(set)
    with open(path) as f:
        for line in f:
            parts = line.rstrip("\n").split("\t")
            if len(parts) != 3:
                continue
            sha, mime, label = parts
            if mime.split(";")[0].strip() in mimes:
                images[sha].add(label)
    return images


def normalised_images(images):
    out = {}
    for sha, labels in images.items():
        out[sha] = {n for n in (normalise(l) for l in labels) if n}
    return out


def image_counts(norm_images):
    counts = Counter()
    for labels in norm_images.values():
        counts.update(labels)
    return counts


def raw_members(images):
    members = defaultdict(Counter)
    for labels in images.values():
        for raw in labels:
            n = normalise(raw)
            if n:
                members[n][raw] += 1
    return members


def cooccurrence_vectors(norm_images, vocab):
    index = {l: i for i, l in enumerate(vocab)}
    n = len(vocab)
    co = np.zeros((n, n), dtype=np.float64)
    counts = np.zeros(n, dtype=np.float64)
    for labels in norm_images.values():
        ids = [index[l] for l in labels if l in index]
        for i in ids:
            counts[i] += 1
            for j in ids:
                if i != j:
                    co[i, j] += 1
    total = max(len(norm_images), 1)
    with np.errstate(divide="ignore", invalid="ignore"):
        pmi = np.log((co * total) / np.outer(counts, counts))
    ppmi = np.nan_to_num(np.maximum(pmi, 0), posinf=0)
    norm = np.linalg.norm(ppmi, axis=1, keepdims=True)
    return ppmi / np.where(norm == 0, 1, norm)


def embed(texts, model_name=EMBED_MODEL):
    import torch
    from transformers import AutoModel, AutoTokenizer

    tok = AutoTokenizer.from_pretrained(model_name)
    model = AutoModel.from_pretrained(model_name).eval()
    out = []
    with torch.no_grad():
        for i in range(0, len(texts), 256):
            batch = tok(texts[i : i + 256], padding=True, truncation=True, return_tensors="pt")
            hidden = model(**batch).last_hidden_state
            mask = batch["attention_mask"].unsqueeze(-1).float()
            vec = (hidden * mask).sum(1) / mask.sum(1)
            out.append(torch.nn.functional.normalize(vec, dim=-1).numpy())
    return np.concatenate(out)


def propose_merges(vocab, counts, text_vecs, ctx_vecs, text_only=0.9, text_min=0.72, ctx_min=0.45):
    text_sim = text_vecs @ text_vecs.T
    ctx_sim = ctx_vecs @ ctx_vecs.T
    order = sorted(range(len(vocab)), key=lambda i: (-counts[vocab[i]], vocab[i]))
    rank = {i: r for r, i in enumerate(order)}
    parent = {}
    for i in order:
        best, best_score = None, 0.0
        for j in order[: rank[i]]:
            t, c = text_sim[i, j], ctx_sim[i, j]
            if t >= text_only or (t >= text_min and c >= ctx_min):
                score = t + c
                if score > best_score:
                    best, best_score = j, score
        if best is not None:
            parent[i] = (best, float(text_sim[i, best]), float(ctx_sim[i, best]))
    heads = {}
    for i in order:
        j = i
        while j in parent:
            j = parent[j][0]
        heads[i] = j
    return heads, parent


def suggest_group(text_vecs, group_vecs):
    return [GROUPS[int(np.argmax(group_vecs @ v))] for v in text_vecs]


def read_merges(path, reviewed_only=False):
    if not path.exists():
        return {}
    with open(path) as f:
        rows = [r for r in csv.DictReader(f) if r["group"].strip() != HARD_NEGATIVE]
    if reviewed_only:
        rows = [r for r in rows if r.get("reviewed", "yes") != "no"]
    return {r["label"]: (r["class"].strip(), r["group"].strip()) for r in rows}


def write_merges(path, rows):
    with open(path, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["label", "class", "group", "images", "text_sim", "ctx_sim", "reviewed"])
        w.writerows(rows)


def resolve(label, target, suggested_group, reviewed):
    if label in reviewed:
        return (*reviewed[label], "yes")
    if target in reviewed:
        cls, group = reviewed[target]
        return cls, group, "via class"
    return target, suggested_group, "no"


def propose(args):
    images = read_images(Path(args.labels))
    norm = normalised_images(images)
    counts = image_counts(norm)
    vocab = sorted(l for l, c in counts.items() if c >= args.candidate_min)
    ctx = cooccurrence_vectors(norm, vocab)
    text = embed(vocab)
    group_vecs = embed([GROUP_PROMPTS[g] for g in GROUPS])
    heads, parent = propose_merges(vocab, counts, text, ctx)
    groups = suggest_group(text, group_vecs)
    reviewed = read_merges(Path(args.decisions))
    rows = []
    for i, label in enumerate(vocab):
        reason = excluded_reason(label)
        target = vocab[heads[i]]
        t, c = (parent[i][1], parent[i][2]) if i in parent else (1.0, 1.0)
        cls, group, status = resolve(label, target, groups[heads[i]], reviewed)
        if reason and label not in reviewed:
            cls, group, status = "-", f"excluded {reason}", "rule"
        rows.append([label, cls, group, counts[label], f"{t:.2f}", f"{c:.2f}", status])
    rows.sort(key=lambda r: (r[1] == "-", r[2], r[1], -r[3]))
    write_merges(Path(args.out), rows)
    print(f"{len(images)} images, {len(counts)} normalised labels, {len(vocab)} candidates written to {args.out}")


def read_hard_negatives(path=DECISIONS):
    if not path.exists():
        return []
    with open(path) as f:
        return [
            (r["label"].strip(), r["class"].strip())
            for r in csv.DictReader(f)
            if r["group"].strip() == HARD_NEGATIVE
        ]


def labels_to_classes(raw_labels, lookup):
    return sorted({lookup[n] for n in (normalise(l) for l in raw_labels) if n in lookup})


def class_lookup(taxonomy):
    lookup = {}
    for c in taxonomy["classes"]:
        for n in c["normalised"]:
            lookup[n] = c["name"]
    return lookup


def class_map(rows):
    return {
        l: (c, g)
        for l, (c, g) in rows.items()
        if c and c != "-" and not excluded_reason(l) and not excluded_reason(c)
    }


def images_per_class(norm, class_of):
    class_images = defaultdict(set)
    for sha, labels in norm.items():
        for l in labels:
            if l in class_of:
                class_images[class_of[l][0]].add(sha)
    group_of = {}
    for label, (cls, group) in class_of.items():
        if label == cls or cls not in group_of:
            group_of[cls] = group
    return class_images, group_of


def build_taxonomy(images, merges, min_images, proposals=None, candidate_min=CANDIDATE_MIN):
    norm = normalised_images(images)
    members = raw_members(images)
    class_of = class_map(merges)
    class_images, group_of = images_per_class(norm, class_of)
    classes = []
    for cls, shas in class_images.items():
        if len(shas) < min_images:
            continue
        normalised = sorted(l for l, (c, _) in class_of.items() if c == cls)
        raw = Counter()
        for n in normalised:
            raw.update(members[n])
        classes.append(
            {
                "name": cls,
                "group": group_of[cls],
                "images": len(shas),
                "normalised": normalised,
                "members": [r for r, _ in raw.most_common()],
            }
        )
    classes.sort(key=lambda c: (GROUPS.index(c["group"]) if c["group"] in GROUPS else len(GROUPS), -c["images"]))
    labelled = sum(1 for labels in norm.values() if any(l in class_of and class_of[l][0] in {c["name"] for c in classes} for l in labels))
    excluded = Counter()
    for label, (cls, group) in merges.items():
        if cls == "-":
            excluded[group] += 1
    kept = {c["name"] for c in classes}
    return {
        "version": VERSION,
        "source_model": "qwen3.5",
        "min_images": min_images,
        "candidate_min": candidate_min,
        "multi_label": "An image gets every class that any of its Qwen labels maps to after normalisation, so one image can have zero to five classes. A label that maps to no class is ignored. Training uses one sigmoid output per class.",
        "excluded": {
            "nsfw": "Nudity, sexual and underwear labels stay with the nsfw model, which has its own human reviewed training set and threshold.",
            "age": "Age words (child, girl, boy, teen and similar) stay with the age model. Mixing them into a general classifier would duplicate the age signal without its review.",
        },
        "counts": {
            "images": len(images),
            "images_with_a_class": labelled,
            "classes": len(classes),
            "excluded_labels": dict(sorted(excluded.items())),
        },
        "groups": [g for g in GROUPS if any(c["group"] == g for c in classes)],
        "classes": classes,
        "candidates": candidate_classes(norm, merges, proposals or {}, kept, candidate_min, min_images),
    }


def candidate_classes(norm, merges, proposals, kept, candidate_min, min_images):
    reviewed = {c for c, _ in merges.values()}
    class_images, group_of = images_per_class(norm, class_map({**proposals, **merges}))
    out = [
        {"name": cls, "group": group_of[cls], "images": len(shas), "reviewed": cls in reviewed}
        for cls, shas in class_images.items()
        if cls not in kept and candidate_min <= len(shas) < min_images
    ]
    return sorted(out, key=lambda c: (-c["images"], c["name"]))


def build(args):
    images = read_images(Path(args.labels))
    merges = read_merges(Path(args.merges), reviewed_only=True)
    proposals = read_merges(Path(args.merges))
    taxonomy = build_taxonomy(images, merges, args.min_images, proposals, args.candidate_min)
    with open(args.out, "w") as f:
        json.dump(taxonomy, f, indent=1, ensure_ascii=False)
        f.write("\n")
    by_group = Counter(c["group"] for c in taxonomy["classes"])
    print(json.dumps(taxonomy["counts"], indent=1))
    for g in taxonomy["groups"]:
        print(f"{by_group[g]:4d} {g}")


FETCH_QUERY = (
    "select lower(hex(u.id)), u.mime_type, l.label from upload_labels l "
    "join uploads u on u.id = l.file where l.model = '{model}'"
)


def fetch(args):
    from export_dataset import kubectl

    raw = kubectl(
        args.namespace,
        args.db_deploy,
        "sh",
        "-c",
        'exec mariadb -uroot -p"$MARIADB_ROOT_PASSWORD" route96 -N -B -e "$0"',
        FETCH_QUERY.format(model=args.model),
    )
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    lines = [l for l in raw.decode().splitlines() if l.count("\t") == 2]
    out.write_text("".join(f"{l}\n" for l in lines))
    images = read_images(out)
    print(f"{len(lines)} label rows, {len(images)} images with labels written to {out}")


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("fetch")
    f.add_argument("--out", default=str(DATA / "labels.tsv"))
    f.add_argument("--model", default="qwen3.5")
    f.add_argument("--namespace", default="nostr-download")
    f.add_argument("--db-deploy", default="mysql")
    a = sub.add_parser("propose")
    a.add_argument("--labels", default=str(DATA / "labels.tsv"))
    a.add_argument("--decisions", default=str(DECISIONS))
    a.add_argument("--out", default=str(MERGES))
    a.add_argument("--candidate-min", type=int, default=CANDIDATE_MIN)
    b = sub.add_parser("build")
    b.add_argument("--labels", default=str(DATA / "labels.tsv"))
    b.add_argument("--merges", default=str(MERGES))
    b.add_argument("--out", default=str(OUT))
    b.add_argument("--min-images", type=int, default=MIN_IMAGES)
    b.add_argument("--candidate-min", type=int, default=CANDIDATE_MIN)
    args = p.parse_args()
    {"fetch": fetch, "propose": propose, "build": build}[args.cmd](args)


if __name__ == "__main__":
    main()
