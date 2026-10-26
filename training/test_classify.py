from export_dataset import classify


def test_explicit_terms_are_positive():
    assert classify(["bed", "nudity", "woman"], False) == ("nsfw", "positive")
    assert classify(["anal_fingering"], False) == ("nsfw", "positive")
    assert classify(["sex_toy"], False) == ("nsfw", "positive")


def test_lingerie_alone_is_ambiguous():
    assert classify(["bed", "lingerie", "woman"], True) is None


def test_benign_breast_terms_are_skipped():
    assert classify(["breast pump"], False) is None


def test_screenshot_is_hard_negative_even_when_baseline_flags():
    assert classify(["screenshot", "text"], True) == ("normal", "hard_negative")


def test_baseline_flag_without_evidence_is_skipped():
    assert classify(["woman", "portrait"], True) is None


def test_plain_image_is_normal():
    assert classify(["cat", "sofa"], False) == ("normal", "normal")


def test_document_and_glass_are_not_explicit():
    assert classify(["document", "wine glass", "grass", "button"], False) == ("normal", "hard_negative")
