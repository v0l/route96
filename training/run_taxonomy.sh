#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
PY=${PY:-$HOME/.venvs/route96-taxonomy/bin/python}
OUT=${OUT:-out/taxonomy}
mkdir -p "$OUT"
for run in "$@"; do
    model=${run%_*}
    loss=${run#*_}
    systemd-run --user --scope -q -p MemoryMax=24G \
        ~/.agents/bin/gpu-run 1 --min-free 2000 -- \
        "$PY" -u train_taxonomy.py --model "$model" --loss "$loss" --out "$OUT" \
        >"$OUT/$run.log" 2>&1
    tail -1 "$OUT/$run.log"
done
