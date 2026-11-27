import json

import torch
from safetensors import safe_open
from safetensors.torch import save_file

from export_head import export


def test_export_writes_head_thresholds_and_class_names(tmp_path):
    best = tmp_path / "run" / "best"
    best.mkdir(parents=True)
    save_file(
        {"classifier.weight": torch.ones(2, 3), "classifier.bias": torch.zeros(2), "vision_model.x": torch.ones(1)},
        best / "model.safetensors",
    )
    (best / "config.json").write_text(json.dumps({"id2label": {"0": "cat", "1": "dog"}}))
    (tmp_path / "run" / "report.json").write_text(json.dumps({"best": {"thresholds": [0.3, 0.6]}}))
    out = tmp_path / "head.safetensors"
    assert tuple(export(tmp_path / "run", out)) == (2, 3)
    with safe_open(out, "pt") as f:
        assert sorted(f.keys()) == ["bias", "thresholds", "weight"]
        assert json.loads(f.metadata()["classes"]) == ["cat", "dog"]
        assert torch.allclose(f.get_tensor("thresholds"), torch.tensor([0.3, 0.6]))
