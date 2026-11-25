import json
from pathlib import Path

import pytest

from export_taxonomy_dataset import (
    check_pairs,
    hard_negatives_for,
    manifest_rows,
    read_manifest,
    select,
    split_shas,
    write_manifest,
)
from taxonomy import read_hard_negatives, read_merges

HERE = Path(__file__).parent

TAXONOMY = {
    "classes": [
        {"name": "cat", "group": "animals", "normalised": ["cat"]},
        {"name": "dog", "group": "animals", "normalised": ["dog"]},
        {"name": "text", "group": "text and documents", "normalised": ["text"]},
    ]
}


def test_select_caps_the_big_class_and_keeps_the_small_one():
    image_classes = {f"t{i:03d}": ["text"] for i in range(50)}
    image_classes.update({f"c{i:03d}": ["cat"] for i in range(5)})
    picked = select(image_classes, min_per_class=3, cap=10, seed=0)
    assert sum(s.startswith("c") for s in picked) == 5
    assert sum(s.startswith("t") for s in picked) == 10


def test_split_matches_the_train_py_rule():
    shas = {f"{i:064x}" for i in range(100)}
    val = split_shas(shas, 0.15, 0)
    assert len(val) == 15
    assert val == split_shas(set(sorted(shas, reverse=True)), 0.15, 0)


def test_hard_negatives_only_from_decisions():
    pairs = [("cat", "dog")]
    assert hard_negatives_for(["cat"], pairs) == ["dog"]
    assert hard_negatives_for(["cat", "dog"], pairs) == []
    assert hard_negatives_for(["text"], pairs) == []
    with pytest.raises(ValueError):
        check_pairs([("cat", "unicorn")], {"cat", "dog"})


def test_manifest_rows_map_labels_and_drop_images_without_a_class(tmp_path):
    labelled = {
        "a": {"Cats", "sofa"},
        "b": {"dog", "Text"},
        "c": {"unicorn"},
    }
    rows = manifest_rows(labelled, TAXONOMY, [("cat", "dog")], 1, 10, 0.0, 0)
    by_sha = {r["sha256"]: r for r in rows}
    assert set(by_sha) == {"a", "b"}
    assert by_sha["a"]["positives"] == "cat"
    assert by_sha["a"]["hard_negatives"] == "dog"
    assert by_sha["b"]["positives"] == "dog|text"
    assert by_sha["b"]["split"] == "train"
    write_manifest(tmp_path / "m.csv", rows)
    assert read_manifest(tmp_path / "m.csv") == rows


def test_committed_hard_negative_decisions_name_taxonomy_classes():
    taxonomy = json.loads((HERE / "taxonomy.json").read_text())
    pairs = read_hard_negatives(HERE / "taxonomy_decisions.csv")
    assert pairs
    check_pairs(pairs, {c["name"] for c in taxonomy["classes"]})
    merges = read_merges(HERE / "taxonomy_decisions.csv")
    assert merges["cat"] == ("cat", "animals")
