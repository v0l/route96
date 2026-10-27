from review import disagree, spread, load_scores


def test_disagree_only_when_verdicts_split_at_half():
    assert disagree({"a": 0.9, "b": 0.1}, ["a", "b"])
    assert not disagree({"a": 0.9, "b": 0.6}, ["a", "b"])
    assert not disagree({"a": 0.1, "b": 0.4}, ["a", "b"])


def test_disagree_restricted_to_chosen_models():
    scores = {"a": 0.9, "b": 0.8, "c": 0.1}
    assert not disagree(scores, ["a", "b"])
    assert disagree(scores, ["a", "c"])


def test_spread_is_max_minus_min():
    assert round(spread({"a": 0.9, "b": 0.2, "c": 0.5}, ["a", "b", "c"]), 2) == 0.7


def test_load_scores_missing_file(tmp_path):
    assert load_scores(tmp_path / "none.csv") == ([], {})


def test_load_scores_reads_columns(tmp_path):
    p = tmp_path / "scores.csv"
    p.write_text("sha256,v3,fal\nabc,0.9,0.1\n")
    names, scores = load_scores(p)
    assert names == ["v3", "fal"]
    assert scores["abc"] == {"v3": 0.9, "fal": 0.1}
