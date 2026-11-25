import torch

from missing_recall import apply_right_list, extras, merge, read_right_list, read_sample, tally, write_sample


def test_right_list_marks_listed_classes_right_and_the_rest_wrong(tmp_path):
    (tmp_path / "r.txt").write_text("0: dog, text\n1:\n")
    rows = [
        {"index": "0", "class": "dog", "verdict": ""},
        {"index": "0", "class": "car", "verdict": ""},
        {"index": "1", "class": "cat", "verdict": ""},
        {"index": "2", "class": "cat", "verdict": ""},
    ]
    got = [r["verdict"] for r in apply_right_list(rows, read_right_list(tmp_path / "r.txt"))]
    assert got == ["right", "wrong", "wrong", ""]


def test_extras_are_predictions_qwen_did_not_give():
    probs = torch.tensor([0.9, 0.7, 0.2, 0.6])
    thresholds = torch.tensor([0.5, 0.5, 0.5, 0.7])
    got = extras(probs, thresholds, {"cat"}, ["cat", "dog", "bird", "car"])
    assert got == [("dog", 0.699999988079071)]


def test_merge_records_which_runs_found_each_extra():
    merged = merge({"a": {"x": [("dog", 0.9)], "y": []}, "b": {"x": [("dog", 0.8), ("car", 0.7)], "y": []}})
    assert merged["x"] == {"dog": ["a", "b"], "car": ["b"]}
    assert merged["y"] == {}


def test_tally_counts_right_per_run_and_group(tmp_path):
    rows = [
        {"index": "0", "sha256": "a", "class": "dog", "runs": "p|q", "verdict": "right"},
        {"index": "0", "sha256": "a", "class": "car", "runs": "q", "verdict": "wrong"},
        {"index": "1", "sha256": "b", "class": "cat", "runs": "p", "verdict": ""},
    ]
    write_sample(tmp_path / "s.csv", rows)
    groups = {"dog": "animals", "cat": "animals", "car": "vehicles"}
    table = tally(read_sample(tmp_path / "s.csv"), groups)
    assert table[("p", "all")] == [1, 1]
    assert table[("q", "all")] == [1, 2]
    assert table[("q", "vehicles")] == [0, 1]
