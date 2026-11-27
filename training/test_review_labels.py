from review_labels import NO_CLASS, format_table, load_review, precision_table, save_review, verdicts_from_wrong_list


def test_wrong_list_marks_the_rest_right():
    rows = [{"sha256": "a", "llm_labels": "cat|sofa"}, {"sha256": "b", "llm_labels": "dog"}, {"sha256": "c", "llm_labels": "x"}]
    out = verdicts_from_wrong_list(rows, "0: sofa\n\n1:\n", 2)
    assert out == {("a", "cat"): "right", ("a", "sofa"): "wrong", ("b", "dog"): "right"}


def test_wrong_list_rejects_unknown_label():
    import pytest

    with pytest.raises(ValueError):
        verdicts_from_wrong_list([{"sha256": "a", "llm_labels": "cat"}], "0: dog", 1)

TAXONOMY = {
    "groups": ["animals", "objects"],
    "classes": [
        {"name": "cat", "group": "animals", "images": 5, "normalised": ["cat"], "members": ["cat"]},
        {"name": "couch", "group": "objects", "images": 5, "normalised": ["couch"], "members": ["sofa"]},
    ],
}


def test_review_round_trip(tmp_path):
    p = tmp_path / "label_review.csv"
    assert load_review(p) == {}
    save_review(p, {("a", "Cats"): "right", ("a", "sofa"): "wrong"})
    assert load_review(p) == {("a", "Cats"): "right", ("a", "sofa"): "wrong"}


def test_precision_groups_by_taxonomy_class():
    review = {
        ("a", "Cats"): "right",
        ("a", "sofa"): "wrong",
        ("b", "cat"): "right",
        ("b", "nude"): "right",
        ("b", "Lindsey Graham"): "wrong",
        ("c", "couch"): "skip",
    }
    tally, per_class = precision_table(review, TAXONOMY)
    assert tally["animals"] == [2, 2]
    assert tally["objects"] == [0, 1]
    assert tally["adult"] == [1, 1]
    assert tally[NO_CLASS] == [0, 1]
    assert tally["all labels"] == [3, 5]
    assert tally["all taxonomy classes"] == [2, 3]
    assert per_class == {"cat": [2, 2], "couch": [0, 1]}


def test_format_table_flags_groups_under_threshold():
    text = format_table({"animals": [9, 10], "objects": [70, 100]})
    assert "| animals | 10 | 9 | 90.0% | keep (few samples) |" in text
    assert "| objects | 100 | 70 | 70.0% | drop or relabel |" in text
