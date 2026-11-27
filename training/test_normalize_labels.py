import numpy as np

from normalize_labels import (
    apply_pair_grades,
    bucket_table,
    centred,
    compact,
    counts_of,
    floors,
    kind,
    may_merge,
    merge_by_meaning,
    precision_by_bin,
    reviewed_map,
    spelling,
    spelling_map,
)


def test_spelling_lowercases_trims_and_singularises():
    assert spelling("  Cat_Ears ") == "cat ear"
    assert spelling("Sunsets.") == "sunset"
    assert spelling("Women") == "woman"


def test_spelling_strips_list_markers_and_articles():
    assert spelling("*   woman") == "woman"
    assert spelling("- text") == "text"
    assert spelling("The Emperor's New Groove") == "emperor's new groove"
    assert spelling("a cat") == "cat"


def test_spelling_keeps_ampersand_as_and():
    assert spelling("Black & White") == "black and white"


def test_hyphen_and_space_variants_join_the_most_used_form():
    files = {"a": {"t-shirt"}, "b": {"t-shirt"}, "c": {"tshirt"}, "d": {"T Shirt"}, "e": {"low poly"}, "f": {"low-poly"}, "g": {"lowpoly"}}
    m = spelling_map(sorted({l for v in files.values() for l in v}), files)
    assert {m["t-shirt"], m["tshirt"], m["T Shirt"]} == {"t-shirt"}
    assert {m["low poly"], m["low-poly"], m["lowpoly"]} == {"low-poly"}
    assert compact("t shirt") == compact("t-shirt") == "tshirt"
    assert compact("emperor's new groove") == compact("theemperorsnewgroove"[3:])
    assert compact("michael j. saylor") == "michaeljsaylor"


def test_counts_are_files_per_label_not_rows():
    files = {"a": {"Cat", "cats"}, "b": {"cat"}}
    assert counts_of(files, {"Cat": "cat", "cats": "cat", "cat": "cat"}) == {"cat": 2}


def test_reviewed_merges_map_members_to_their_class():
    assert reviewed_map({"Kitten": "cat", "Dog": "dog"}, {"cat": "cat"}) == {"Kitten": "cat", "Dog": "dog"}
    assert reviewed_map({"x": "female"}, {"female": "woman"}) == {"x": "woman"}


def test_kind_marks_age_adult_and_both():
    assert kind("schoolgirl") == "age"
    assert kind("lingerie") == "adult"
    assert kind("nude girl") == "age and adult"
    assert kind("sunset") == ""


def test_age_and_adult_labels_never_merge_across():
    assert not may_merge("nude girl", "nude", {}, set())
    assert not may_merge("nude", "nude girl", {}, set())
    assert not may_merge("girl", "woman", {}, set())
    assert not may_merge("lingerie", "dress", {}, set())
    assert not may_merge("dress", "lingerie", {}, set())
    assert may_merge("naked", "nude", {}, set())
    assert may_merge("young girl", "girl", {}, set())


def test_labels_with_different_numbers_never_merge():
    assert not may_merge("rollercoaster tycoon 3", "rollercoaster tycoon 2", {}, set())
    assert not may_merge("2026", "2025", {}, set())
    assert not may_merge("rollercoaster tycoon 2", "rollercoaster tycoon", {}, set())
    assert may_merge("formula 1 car", "formula 1", {}, set())


def test_spelling_strips_numbered_list_markers():
    assert spelling("1. Typography") == "typography"
    assert spelling("5. potion") == "potion"
    assert spelling("3d render") == "3d render"


def test_pair_grades_and_precision_by_bin():
    rows = [{"bin": "0.80", "same": ""}, {"bin": "0.80", "same": ""}, {"bin": "0.90", "same": ""}]
    rows = apply_pair_grades(rows, {0, 2})
    assert [r["same"] for r in rows] == ["yes", "no", "yes"]
    assert precision_by_bin(rows) == {"0.80": (1, 2), "0.90": (1, 1)}
    rows[1]["allowed"] = "no"
    assert precision_by_bin(rows) == {"0.80": (1, 1), "0.90": (1, 1)}


def test_roman_numerals_count_as_numbers():
    assert not may_merge("mafia iii", "mafia ii", {}, set())
    assert not may_merge("rocky iv", "rocky", {}, set())
    assert may_merge("mafia ii game", "mafia ii", {}, set())


def test_nudity_levels_never_merge():
    assert not may_merge("partial nudity", "explicit nudity", {}, set())
    assert not may_merge("full nudity", "nudity", {}, set())
    assert may_merge("naked", "nude", {}, set())


def test_platforms_and_post_types_never_merge():
    assert not may_merge("instagram post", "twitter post", {}, set())
    assert not may_merge("instagram story", "instagram post", {}, set())
    assert not may_merge("facebook reel", "instagram reel", {}, set())
    assert may_merge("instagram post screenshot", "instagram post", {}, set())


def test_garment_lengths_never_merge():
    assert not may_merge("knee-high socks", "thigh-high socks", {}, set())
    assert not may_merge("ankle socks", "knee socks", {}, set())


def test_search_classes_stay_apart():
    for a, b in [("blog", "url"), ("cryptocurrency trading", "cryptocurrency"), ("cryptocurrency exchange", "cryptocurrency"), ("cryptocurrency exchange", "cryptocurrency trading"), ("england", "united kingdom")]:
        assert not may_merge(a, b, {}, set())
        assert not may_merge(b, a, {}, set())


def test_taxonomy_classes_and_hard_negatives_stay_apart():
    class_of = {"cat": "cat", "dog": "dog"}
    assert not may_merge("cat", "dog", class_of, set())
    assert not may_merge("cat", "kitty", class_of, set())
    assert may_merge("kitty", "cat", class_of, set())
    assert not may_merge("indoor", "outdoor", {}, {("indoor", "outdoor")})
    assert not may_merge("outdoor", "indoor", {}, {("indoor", "outdoor")})


def test_meaning_merge_joins_close_labels_into_the_most_used():
    labels = ["sunset", "sunset sky", "sky at sunset", "cat"]
    counts = {"sunset": 50, "sunset sky": 5, "sky at sunset": 2, "cat": 40}
    v = np.array([[1.0, 0.0], [0.99, 0.14], [0.98, 0.2], [0.0, 1.0]])
    v = v / np.linalg.norm(v, axis=1, keepdims=True)
    head, best = merge_by_meaning(labels, counts, v, 0.95, lambda a, b: True)
    assert head["sunset sky"][0] == "sunset"
    assert head["sky at sunset"][0] == "sunset"
    assert head["cat"] == ("cat", 1.0)
    assert best["cat"][0] == "sunset"


def test_meaning_merge_only_joins_heads_and_respects_the_guard():
    labels = ["a", "b", "c"]
    counts = {"a": 10, "b": 5, "c": 1}
    v = np.array([[1.0, 0.0], [0.96, 0.28], [0.85, 0.53]])
    head, _ = merge_by_meaning(labels, counts, v, 0.95, lambda x, y: True)
    assert head["b"][0] == "a"
    assert head["c"][0] == "c"
    head, _ = merge_by_meaning(labels, counts, v, 0.95, lambda x, y: x != "b")
    assert head["b"] == ("b", 1.0)


def test_centred_vectors_are_unit_length():
    v = centred(np.random.default_rng(0).normal(size=(10, 4)) + 5)
    assert np.allclose(np.linalg.norm(v, axis=1), 1.0)


def test_buckets_and_floors():
    counts = {"a": 1, "b": 3, "c": 12, "d": 600}
    rows = {r["bucket"]: r for r in bucket_table(counts)}
    assert rows["1"]["labels"] == 1
    assert rows["2-4"]["uses"] == 3
    assert rows["500+"]["share"] == 600 / 616
    assert floors(counts) == {10: 2, 20: 1, 30: 1, 50: 1}
