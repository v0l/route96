import os
from pathlib import Path

ENV = "ROUTE96_TAXONOMY_ROOT"
DATASET = "taxonomy_v2"


def root(env=None):
    env = os.environ if env is None else env
    return Path(env.get(ENV) or Path.home() / "route96-taxonomy").expanduser()


ROOT = root()
DATA = ROOT / "data" / DATASET
OUT = ROOT / "out" / "taxonomy"
LABELS = ROOT / "data" / "taxonomy"
