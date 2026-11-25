import torch

from multilabel import (
    LabelSpace,
    best_thresholds,
    evaluate,
    hard_negative_rate,
    partial_weights,
    plain_weights,
    weighted_bce,
)

TAXONOMY = {
    "classes": [
        {"name": "cat", "group": "animals"},
        {"name": "dog", "group": "animals"},
        {"name": "bird", "group": "animals"},
        {"name": "text", "group": "text"},
        {"name": "chart", "group": "text"},
        {"name": "car", "group": "vehicles"},
    ]
}


def test_label_space_groups_and_vectors():
    space = LabelSpace(TAXONOMY)
    pos = torch.stack([space.vector(["cat"]), space.vector(["text", "car"])])
    near = space.same_group(pos)
    assert near[0].tolist() == [True, True, True, False, False, False]
    assert near[1].tolist() == [False, False, False, True, True, True]


def test_partial_weights_count_positive_near_and_sampled_far_only():
    space = LabelSpace(TAXONOMY)
    pos = space.vector(["cat"]).unsqueeze(0)
    hard = space.vector(["car"]).unsqueeze(0)
    w = partial_weights(pos, hard, space.same_group(pos), far_samples=0)
    assert w[0].tolist() == [1.0, 1.0, 1.0, 0.0, 0.0, 1.0]
    w = partial_weights(pos, torch.zeros_like(pos), space.same_group(pos), far_samples=10, far_weight=0.1)
    assert torch.allclose(w[0], torch.tensor([1.0, 1.0, 1.0, 0.1, 0.1, 0.1]))


def test_partial_weights_sample_about_the_requested_far_count():
    names = [{"name": f"c{i}", "group": f"g{i}"} for i in range(200)]
    space = LabelSpace({"classes": names})
    pos = space.vector(["c0"]).unsqueeze(0).repeat(500, 1)
    g = torch.Generator().manual_seed(0)
    w = partial_weights(pos, torch.zeros_like(pos), space.same_group(pos), generator=g, far_samples=16)
    far = (w[:, 1:] > 0).sum(1).float().mean()
    assert 14 < far < 18


def test_missing_class_costs_nothing_with_partial_loss():
    space = LabelSpace(TAXONOMY)
    pos = space.vector(["cat"]).unsqueeze(0)
    logits = torch.tensor([[4.0, -4.0, -4.0, 6.0, -4.0, -4.0]])
    w = partial_weights(pos, torch.zeros_like(pos), space.same_group(pos), far_samples=0)
    assert weighted_bce(logits, pos, w) < 0.05
    assert weighted_bce(logits, pos, plain_weights(pos)) > 0.9


def test_thresholds_and_metrics():
    probs = torch.tensor([[0.9, 0.2], [0.6, 0.1], [0.3, 0.8], [0.1, 0.4]])
    truth = torch.tensor([[1.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.0, 1.0]])
    t = best_thresholds(probs, truth)
    assert 0.3 < t[0] <= 0.6
    assert 0.2 < t[1] <= 0.4
    report = evaluate(probs, truth, t, ["a", "b"])
    assert report["macro_f1"] == 1.0
    assert report["top5_hit"] == 1.0
    assert report["per_class"][0]["support"] == 2
    hard = torch.tensor([[0.0, 1.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]])
    assert hard_negative_rate(probs, hard, t) == 0.0


def test_report_uses_held_out_thresholds(tmp_path):
    from train_taxonomy import read_rows, report

    space = LabelSpace(TAXONOMY)
    (tmp_path / "manifest.csv").write_text(
        "sha256,split,positives,hard_negatives,llm_labels\n"
        "a,train,cat,dog,cats\n"
        "b,val,dog|car,,dog|car\n"
    )
    train, val = read_rows(tmp_path, space)
    assert [r[0] for r in train] == ["a"] and [r[0] for r in val] == ["b"]
    assert train[0][3].tolist() == space.vector(["dog"]).tolist()
    g = torch.Generator().manual_seed(0)
    truth = (torch.rand(200, len(space), generator=g) < 0.3).float()
    probs = (truth * 0.6 + torch.rand(200, len(space), generator=g) * 0.4).clamp(0, 1)
    r = report(probs, truth, torch.zeros_like(truth), space)
    assert r["macro_f1"] > 0.9
    assert len(r["thresholds"]) == len(space)
    assert r["hard_negative_rate"] is None
