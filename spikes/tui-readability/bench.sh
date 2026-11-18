#!/usr/bin/env bash
# Headless measurements: frame time and bytes per frame (200x50 viewport over a counting sink,
# scripted scroll of 3 rows a frame through the whole fixture) and RSS at three points, with the
# highlighter feature on and off. Needs fixtures/ (./fetch-fixtures.sh). Output: out/bench.txt.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p out
cargo build --release
cargo build --release --no-default-features --target-dir target/nohl
{
    for fx in v2-rework tui-return skills-grill; do
        for v in a b c; do
            for osc in on off; do
                ./target/release/tui-readability-spike --bench --variant $v --osc $osc --fixture fixtures/$fx.jsonl
            done
        done
    done
    # Highlighter feature off: the RSS baseline.
    for v in a b c; do
        echo -n "[feature off] "
        ./target/nohl/release/tui-readability-spike --bench --variant $v --osc on --fixture fixtures/v2-rework.jsonl
    done
    echo -n "[hl toggled off at run time] "
    ./target/release/tui-readability-spike --bench --variant b --osc on --no-highlight --fixture fixtures/v2-rework.jsonl
} | tee out/bench.txt
ls -l target/release/tui-readability-spike target/nohl/release/tui-readability-spike
