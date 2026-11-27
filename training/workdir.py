import os
from pathlib import Path

ENV = "ROUTE96_TAXONOMY_ROOT"
DATASET = "taxonomy_v2"
DEFAULT_ROOT = "/core/datasets/route96-taxonomy"


def root(env=None):
    env = os.environ if env is None else env
    return Path(env.get(ENV) or DEFAULT_ROOT).expanduser()


ROOT = root()
DATA = ROOT / "data" / DATASET
OUT = ROOT / "out" / "taxonomy"
LABELS = ROOT / "data" / "taxonomy"
