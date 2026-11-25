import json
from pathlib import Path

import numpy as np

from taxonomy import (
    build_taxonomy,
    class_lookup,
    excluded_reason,
    image_counts,
    labels_to_classes,
    normalise,
    normalised_images,
    propose_merges,
    read_images,
    resolve,
)

HERE = Path(__file__).parent


def test_normalise_case_underscores_and_plurals():
    assert normalise("Cat_Ears") == "cat ear"
    assert normalise("cat ears") == "cat ear"
    assert normalise("Cartoon") == "cartoon"
    assert normalise("abstract_art") == "abstract"
    assert normalise("Trees") == "tree"
    assert normalise("women") == "woman"
    assert normalise("  dark_theme ") == "dark mode"


def test_normalise_keeps_words_that_are_not_plurals():
    for word in ["glasses", "news", "christmas", "mathematics", "jeans", "dress", "bus", "politics"]:
        assert normalise(word) == word
    assert normalise("abdomen") == "abdomen"
    assert normalise("outdoors") == "outdoor"


def test_normalise_empty_and_punctuation():
    assert normalise("  ") == ""
    assert normalise("\"meme.\"") == "meme"


def test_excluded_reason_routes_nsfw_and_age():
    assert excluded_reason("nude woman") == "nsfw"
    assert excluded_reason("lingerie") == "nsfw"
    assert excluded_reason("anime girl") == "age"
    assert excluded_reason("child") == "age"
    assert excluded_reason("cat") is None
    assert excluded_reason("bass guitar") is None


def test_read_images_keeps_only_images(tmp_path):
    p = tmp_path / "labels.tsv"
    p.write_text("a\timage/jpeg\tcat\na\timage/jpeg\tCats\nb\tvideo/mp4\tdog\nc\timage/webp\tdog\nbroken\n")
    images = read_images(p)
    assert set(images) == {"a", "c"}
    assert normalised_images(images)["a"] == {"cat"}
    assert image_counts(normalised_images(images)) == {"cat": 1, "dog": 1}


def test_propose_merges_needs_text_and_context():
    vocab = ["cat", "kitty", "dog"]
    counts = {"cat": 10, "kitty": 3, "dog": 8}
    text = np.array([[1.0, 0.0], [0.8, 0.6], [0.0, 1.0]])
    ctx_close = np.array([[1.0, 0.0], [1.0, 0.0], [0.0, 1.0]])
    heads, parent = propose_merges(vocab, counts, text, ctx_close)
    assert vocab[heads[1]] == "cat"
    assert heads[0] == 0 and heads[2] == 2
    ctx_far = np.array([[1.0, 0.0], [0.0, 1.0], [0.0, 1.0]])
    heads, _ = propose_merges(vocab, counts, text, ctx_far)
    assert heads[1] == 1


def test_resolve_prefers_reviewed_label_then_reviewed_class():
    reviewed = {"sofa": ("couch", "objects"), "couch": ("couch", "objects")}
    assert resolve("sofa", "x", "people", reviewed) == ("couch", "objects", "yes")
    assert resolve("settee", "couch", "people", reviewed) == ("couch", "objects", "via class")
    assert resolve("lamp", "lamp", "objects", reviewed) == ("lamp", "objects", "no")


def test_build_taxonomy_counts_images_and_multi_label():
    images = {
        "a": {"cat", "Sofa", "text"},
        "b": {"cats", "couch"},
        "c": {"text", "nude"},
    }
    merges = {
        "cat": ("cat", "animals"),
        "couch": ("couch", "objects"),
        "text": ("text", "text and documents"),
        "nude": ("-", "excluded nsfw"),
    }
    t = build_taxonomy(images, merges, min_images=2)
    by_name = {c["name"]: c for c in t["classes"]}
    assert set(by_name) == {"cat", "couch", "text"}
    assert by_name["cat"]["images"] == 2
    assert sorted(by_name["cat"]["members"]) == ["cat", "cats"]
    assert t["counts"]["images_with_a_class"] == 3
    lookup = class_lookup(t)
    assert labels_to_classes(["Cats", "sofa", "Text", "nude"], lookup) == ["cat", "couch", "text"]
    assert labels_to_classes(["nude"], lookup) == []


def test_min_images_drops_small_classes():
    images = {"a": {"cat"}, "b": {"dog"}, "c": {"dog"}}
    merges = {"cat": ("cat", "animals"), "dog": ("dog", "animals")}
    t = build_taxonomy(images, merges, min_images=2)
    assert [c["name"] for c in t["classes"]] == ["dog"]


def test_committed_taxonomy_is_consistent():
    t = json.loads((HERE / "taxonomy.json").read_text())
    names = [c["name"] for c in t["classes"]]
    assert len(names) == len(set(names))
    assert 150 <= len(names) <= 300
    seen = set()
    for c in t["classes"]:
        assert c["group"] in t["groups"]
        assert c["images"] >= t["min_images"]
        assert not excluded_reason(c["name"])
        assert not seen & set(c["normalised"])
        seen |= set(c["normalised"])
