from collections import Counter

from heldout import agreement, apply_grades, disagreements, parse_labelled, parse_predictions, pick, sample_disagreements, scores, tally


def test_grades_set_verdicts_by_index():
    rows = [{"index": "0", "verdict": ""}, {"index": "10", "verdict": ""}, {"index": "1", "verdict": ""}]
    assert [r["verdict"] for r in apply_grades(rows, "0m 10q")] == ["model", "qwen", ""]

LOOKUP = {"cat": "cat", "kitten": "cat", "dog": "dog"}


def test_pick_skips_training_images_and_images_without_a_class():
    labelled = {"a": ["cat"], "b": ["dog"], "c": ["car"], "d": ["kitten"]}
    got = pick(labelled, LOOKUP, {"b"}, 10, 0)
    assert sorted(got) == ["a", "d"]
    assert pick(labelled, LOOKUP, set(), 10, 3) == pick(labelled, LOOKUP, set(), 10, 3)


def test_parse_labelled_reads_the_mariadb_rows():
    assert parse_labelled("ab\timage/png\tcat|dog\nbad line\n") == {"ab": ["cat", "dog"]}


def test_predictions_skip_errors_and_keep_probabilities(tmp_path):
    p = tmp_path / "p.tsv"
    p.write_text("file\tmime\tms\tlabels\na\timage/png\t10\tcat=0.9000|dog=0.2000\nb\timage/png\t5\t\nc\timage/png\t1\terror: x\nd\timage/png\t1\terror message=0.8000\n")
    assert parse_predictions(p) == {"a": {"cat": 0.9, "dog": 0.2}, "b": {}, "d": {"error message": 0.8}}


def test_agreement_counts_and_scores():
    truth = {"a": {"cat"}, "b": {"dog"}, "c": set()}
    preds = {"a": {"cat": 0.9}, "b": {"cat": 0.6}, "c": {}}
    table = agreement(truth, preds, ["cat", "dog"])
    assert table["cat"] == Counter(tp=1, fp=1, tn=1)
    assert table["dog"] == Counter(fn=1, tn=2)
    s = scores(table["cat"])
    assert (s["qwen"], s["model"], s["both"], s["precision"], s["recall"]) == (1, 2, 1, 0.5, 1.0)
    assert scores(table["dog"])["precision"] is None
    assert scores(Counter(tn=3))["f1"] is None


def test_disagreements_list_both_sides_and_sample_is_stable():
    rows = disagreements({"a": {"cat"}, "b": set()}, {"a": {"dog": 0.7}, "b": {"cat": 0.5}})
    assert rows == [("a", "dog", "model only", 0.7), ("a", "cat", "qwen only", None), ("b", "cat", "model only", 0.5)]
    assert sample_disagreements(rows, 2, 1) == sample_disagreements(rows, 2, 1)
    assert len(sample_disagreements(rows, 10, 1)) == 3


def test_tally_ignores_ungraded_rows():
    rows = [
        {"side": "model only", "verdict": "model"},
        {"side": "model only", "verdict": "qwen"},
        {"side": "qwen only", "verdict": "unclear"},
        {"side": "qwen only", "verdict": ""},
    ]
    assert tally(rows) == {"model only": {"model": 1, "qwen": 1}, "qwen only": {"unclear": 1}}
