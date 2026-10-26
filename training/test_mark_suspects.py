from mark_suspects import mark


def rows():
    return [
        {"sha256": "a", "kind": "normal"},
        {"sha256": "b", "kind": "normal"},
        {"sha256": "c", "kind": "positive"},
        {"sha256": "d", "kind": "hard_negative"},
    ]


def scores(**v):
    return {k: {"sha256": k, "ours": s[0], "falcon": s[1]} for k, s in v.items()}


def test_any_model_over_threshold_marks_negatives_only():
    r = rows()
    s = scores(a=("0.1", "0.9"), b=("0.1", "0.2"), c=("0.9", "0.9"), d=("0.6", "0.0"))
    assert mark(r, s, 0.5) == 2
    assert [x["kind"] for x in r] == ["suspect", "normal", "positive", "suspect"]
