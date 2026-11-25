import torch

from missing_recall import extras, tally


def test_extras_are_predictions_qwen_did_not_give():
    probs = torch.tensor([0.9, 0.7, 0.2, 0.6])
    thresholds = torch.tensor([0.5, 0.5, 0.5, 0.7])
    got = extras(probs, thresholds, {"cat"}, ["cat", "dog", "bird", "car"])
    assert got == [("dog", 0.699999988079071)]


def test_tally_counts_right_per_group():
    verdicts = {("a", "dog"): "right", ("a", "car"): "wrong", ("b", "dog"): "right", ("b", "cat"): ""}
    groups = {"dog": "animals", "cat": "animals", "car": "vehicles"}
    assert tally(verdicts, groups) == {"all": [2, 3], "animals": [2, 2], "vehicles": [0, 1]}
