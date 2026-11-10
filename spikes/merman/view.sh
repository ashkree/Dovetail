#!/usr/bin/env bash
# S5: show the merman renders in kitty. Run from the repo root.
set -euo pipefail
dir=docs/spikes/merman
for f in legend.png crop-plain-applyfn.png crop-selected-applyfn.png map.png; do
  echo "== $f"
  kitten icat "$dir/$f"
done
