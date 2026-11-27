import torch

from floors import Space, export_head, file_counts, floor_classes, normalized_labels, subset_scores, tier_of, train_head


def test_floor_classes_keep_base_first_and_skip_age_and_dropped():
    counts = {"cat": 90, "rose": 40, "girl": 60, "rope": 56, "kanji": 12, "nude girl": 30}
    kinds = {"girl": "age", "nude girl": "age and adult"}
    assert floor_classes(counts, ["cat"], 30, kinds, {"rope"}) == ["cat", "rose"]
    assert floor_classes(counts, ["cat"], 10, kinds, {"rope"}) == ["cat", "rose", "kanji"]


def test_tiers_name_the_floor_a_class_first_appears_at():
    counts = {"cat": 90, "rose": 40, "kanji": 12, "tea": 25}
    assert tier_of("cat", counts, {"cat"}) == "base"
    assert tier_of("rose", counts, {"cat"}) == "30+"
    assert tier_of("tea", counts, {"cat"}) == "20+"
    assert tier_of("kanji", counts, {"cat"}) == "10+"


def test_file_counts_map_raw_labels_and_keep_images_only(tmp_path):
    p = tmp_path / "labels.tsv"
    p.write_text("a\timage/png\tCats\na\timage/png\tcat\nb\timage/jpeg\tcat\nc\tvideo/mp4\tcat\nd\timage/png\tunknown\n")
    assert file_counts(p, {"Cats": "cat", "cat": "cat"}) == {"cat": 2}


def test_normalized_labels_and_space_vector():
    assert normalized_labels(["Cats", "x", ""], {"Cats": "cat"}) == {"cat"}
    s = Space(["cat", "dog"])
    assert s.vector({"dog", "fish"}).tolist() == [0.0, 1.0]


def test_subset_scores_average_only_classes_with_validation_images():
    per = [
        {"class": "a", "support": 2, "f1": 0.5, "precision": 0.4, "recall": 0.6},
        {"class": "b", "support": 0, "f1": 0.0, "precision": 0.0, "recall": 0.0},
        {"class": "c", "support": 1, "f1": 1.0, "precision": 1.0, "recall": 1.0},
    ]
    got = subset_scores(per, {"a", "b"})
    assert got == {"classes": 2, "with_val": 1, "macro_f1": 0.5, "macro_precision": 0.4, "macro_recall": 0.6}
    assert subset_scores(per, {"b"})["macro_f1"] is None


def test_export_head_writes_the_server_head_format(tmp_path):
    from safetensors import safe_open
    import json

    saved = {"names": ["cat", "rose"], "state": {"weight": torch.ones(2, 3), "bias": torch.zeros(2)}, "thresholds": [0.3, 0.6]}
    out = tmp_path / "head.safetensors"
    assert export_head(saved, out) == (2, 3)
    with safe_open(str(out), "pt") as f:
        assert json.loads(f.metadata()["classes"]) == ["cat", "rose"]
        assert f.get_tensor("thresholds").tolist() == [0.30000001192092896, 0.6000000238418579]
    saved["thresholds"] = [0.3]
    try:
        export_head(saved, out)
        assert False
    except ValueError:
        pass


def test_train_head_learns_a_separable_problem():
    g = torch.Generator().manual_seed(0)
    x = torch.randn(256, 8, generator=g)
    y = torch.stack([(x[:, 0] > 0).float(), (x[:, 1] > 0).float()], 1)
    last = None
    for _, _, probs in train_head(x, y, x, y, 30, 32, 0.05, "cpu"):
        last = probs
    assert last is not None
    assert ((last > 0.5).float() == y).float().mean() > 0.95
