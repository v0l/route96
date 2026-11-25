from taxonomy_report import summary_table, weakest_table, write_per_class


def report(f1s):
    per_class = [
        {"class": c, "support": 3, "predicted": 2, "precision": f, "recall": f, "f1": f, "threshold": 0.5}
        for c, f in f1s.items()
    ]
    best = {
        "macro_f1": 0.5, "micro_f1": 0.6, "macro_precision": 0.5, "macro_recall": 0.5, "top5_hit": 0.9,
        "top5_recall": 0.7, "predicted_per_image": 3.0, "hard_negative_rate": 0.1, "per_class": per_class,
    }
    return {"best": best, "throughput": [{"images_per_second": 1000.0}, {"ms_per_image": 2.0}]}


def test_weakest_lists_lowest_f1_with_the_other_run():
    plain = report({"cat": 0.9, "dog": 0.1, "car": 0.5})
    partial = report({"cat": 0.8, "dog": 0.3, "car": 0.4})
    table = weakest_table(plain, 2, partial).splitlines()
    assert table[2].startswith("| dog |") and table[2].endswith("| 0.30 |")
    assert table[3].startswith("| car |")
    assert len(table) == 4


def test_summary_and_csv(tmp_path):
    reports = {"siglip_plain": report({"cat": 0.9})}
    assert "| siglip_plain | 0.500 |" in summary_table(reports)
    path = write_per_class(tmp_path, reports)
    assert path.read_text().splitlines()[1].startswith("siglip_plain,cat,3,2,0.9")
