import torch

from openvocab import agreement, fill, head_in_vocabulary, hybrid, label_sets, pick, recall_outside, scores, tensor_agreement, truth_matrix, tune, vocabulary


def test_vocabulary_skips_age_kinds_and_dropped_labels():
    kinds = {"cat": "", "girl": "age", "nude": "adult", "adult content": "age and adult", "rope": ""}
    assert vocabulary(kinds, {"rope"}) == ["cat", "nude"]


def test_templates_fill_the_label():
    assert fill("bare", "cat") == "cat"
    assert fill("photo", "cat") == "a photo of cat."


def test_scores_are_sigmoid_of_scaled_cosine():
    img = torch.tensor([[2.0, 0.0]])
    text = torch.tensor([[1.0, 0.0], [0.0, 1.0]])
    got = scores(img, text, 10.0, -5.0)
    assert torch.allclose(got, torch.sigmoid(torch.tensor([[5.0, -5.0]])))


def test_pick_applies_threshold_and_top_k():
    p = torch.tensor([[0.9, 0.5, 0.4, 0.1]])
    assert pick(p, 0.3, 0).tolist() == [[True, True, True, False]]
    assert pick(p, 0.3, 2).tolist() == [[True, True, False, False]]


def test_agreement_counts_sets():
    a = agreement([{"cat", "dog"}, {"car"}], [{"cat"}, {"car", "road"}])
    assert (a["both"], a["model"], a["qwen"]) == (2, 3, 3)
    assert a["precision"] == round(2 / 3, 4) and a["recall"] == round(2 / 3, 4)


def test_tensor_agreement_matches_set_agreement_and_counts_truth_outside_the_vocabulary():
    names = ["cat", "dog", "car"]
    truth = [{"cat"}, {"car", "road"}]
    pred = label_sets(torch.tensor([[True, True, False], [False, False, True]]), names)
    t = truth_matrix(truth, names) > 0
    got = tensor_agreement(torch.tensor([[True, True, False], [False, False, True]]), t, extra_truth=1)
    assert got == agreement(pred, truth)


def test_hybrid_keeps_head_and_adds_zero_shot_only_outside_head_classes():
    head = [{"cat"}, set()]
    zero = [{"cat", "kitten", "dog"}, {"car"}]
    assert hybrid(head, zero, ["cat", "dog"]) == [{"cat", "kitten"}, {"car"}]


def test_head_in_vocabulary_places_head_columns():
    pred, covered = head_in_vocabulary(torch.tensor([[True, False]]), ["dog", "cat"], ["cat", "car", "dog"])
    assert pred.tolist() == [[False, False, True]]
    assert covered.tolist() == [True, False, True]


def test_tune_picks_the_best_threshold_and_respects_the_head():
    probs = torch.tensor([[0.9, 0.2], [0.1, 0.8]])
    truth = torch.tensor([[True, False], [False, True]])
    a, t, k = tune(probs, truth, torch.tensor([0.05, 0.5]), [0])
    assert a["f1"] == 1.0 and t == 0.5
    base = torch.tensor([[False, False], [False, True]])
    a, _, _ = tune(probs, truth, torch.tensor([0.5]), [0], base=base, covered=torch.tensor([False, True]))
    assert a["f1"] == 1.0


def test_recall_outside_counts_only_labels_the_head_does_not_cover():
    got = recall_outside([{"cat", "kitten"}, {"car"}], [{"cat", "kitten"}, {"road"}], {"cat"})
    assert got == {"qwen": 2, "found": 1, "recall": 0.5}
