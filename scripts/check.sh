#!/usr/bin/env bash
# The wave gate (AGENTS.md "How to work" 3), plus mechanical checks for rules agents keep missing.
# Run before reporting a wave or a fix done. Exits non-zero on the first failure.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --check
# --workspace takes in the GUI shell (gui/src-tauri), which default-members leaves out of plain cargo runs.
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# The Tauri window loads the Vite dev server: one port, the same in both files (gui/DECISIONS.md 1).
vite_port=$(grep -E '^ *server:' gui/vite.config.ts | grep -oE 'port: [0-9]+' | grep -oE '[0-9]+')
tauri_port=$(grep -oE '"devUrl": "http://127.0.0.1:[0-9]+"' gui/src-tauri/tauri.conf.json | grep -oE '[0-9]+"$' | tr -d '"')
if [[ -z $vite_port || $vite_port != "$tauri_port" ]]; then
    echo "check.sh: gui/vite.config.ts port ($vite_port) and tauri.conf.json devUrl port ($tauri_port) differ" >&2
    exit 1
fi

# ADR 0023: the GUI's TypeScript types are generated from dovetail-api and committed. Regenerate
# them; a Rust change the GUI has not taken in shows as a difference.
cargo test -q -p dovetail-api --features ts --test ts_export >/dev/null
if [[ -n $(git status --porcelain -- gui/src/core/generated) ]]; then
    echo "check.sh: gui/src/core/generated is out of date with dovetail-api; commit the regenerated files:" >&2
    git status --short -- gui/src/core/generated >&2
    exit 1
fi

# The GUI frontend's own gate: type check, unit tests (its apply() against the shared cases in
# crates/dovetail-api/tests/fixtures/apply), production build.
if [[ ! -d gui/node_modules ]]; then
    echo "check.sh: gui/node_modules is missing; run: npm --prefix gui install" >&2
    exit 1
fi
npm --prefix gui run --silent check
# Also enforced by `cargo test` (CODING_STANDARDS.md "Architecture", schema changes go through
# migrations): migrations are contiguous and listed in order
# (dovetail-core `db::tests::migrations_are_contiguous_and_listed_in_order`).

# Parallel branches each take "the next" migration number; the second to merge clashes. A migration
# this branch adds must not reuse a number main already has under another name. Offline, the last
# fetched origin/main is used.
git fetch -q origin main 2>/dev/null || true
if git rev-parse -q --verify origin/main >/dev/null; then
    mig=crates/dovetail-core/src/db/migrations
    clashes=""
    for db in state registry; do
        main_files=$(git ls-tree --name-only origin/main -- "$mig/$db/" | xargs -rn1 basename)
        for f in "$mig/$db"/*.sql; do
            name=$(basename "$f")
            grep -qxF "$name" <<<"$main_files" && continue
            num=${name%%_*}
            if taken=$(grep -E "^${num}_" <<<"$main_files"); then
                clashes+="$db/$name: main already has $db/$taken"$'\n'
            fi
        done
    done
    if [[ -n $clashes ]]; then
        echo "check.sh: migration numbers clash with origin/main; renumber yours after main's last:" >&2
        printf '%s' "$clashes" >&2
        exit 1
    fi
fi

# CODING_STANDARDS.md "Code": typed (thiserror) errors in libraries; String errors only in the binary.
libs=(crates/dovetail-api crates/dovetail-core crates/dovetail-driver crates/dovetail-maps crates/dovetail-mock crates/dovetail-tui)
if hits=$(grep -rnE 'Result<[^;{]*,[[:space:]]*String>' "${libs[@]/%//src}"); then
    echo "check.sh: library code returns String errors; use a thiserror type:" >&2
    echo "$hits" >&2
    exit 1
fi

# CODING_STANDARDS.md "Architecture": the core / UI boundary is message-passing (decision 15). The TUI
# sends Commands; it never spawns processes or writes files itself. Test modules are skipped: by
# convention each file's `#[cfg(test)]` module comes last, so everything after it is ignored.
tui_banned='std::process|Command::new|fs::write|File::create|create_dir|remove_file|OpenOptions'
hits=$(find crates/dovetail-tui/src -name '*.rs' -exec awk -v pat="$tui_banned" '
        FNR == 1 { in_test = 0 }
        /^#\[cfg\(test\)\]/ { in_test = 1 }
        !in_test && $0 ~ pat { print FILENAME ":" FNR ": " $0 }
    ' {} +)
if [[ -n $hits ]]; then
    echo "check.sh: the TUI spawns processes or writes files; send a Command to the core instead:" >&2
    echo "$hits" >&2
    exit 1
fi

# AGENTS.md "How to work" 6: never add a dependency without stating why. Every external crate in
# [workspace.dependencies], and every direct non-workspace dependency of a member (dev and build
# included), needs a row in the dependency table of docs/implementation-plan.md.
plan_deps=$(awk -F'|' '
        /^\| Dependency \|/ { in_table = 1; next }
        in_table && !/^\|/ { exit }
        in_table { print $2 }
    ' docs/implementation-plan.md | grep -o '`[^`]*`' | tr -d '`' | sort -u)
declared=$(awk '
        /^\[/ {
            in_ws = ($0 == "[workspace.dependencies]")
            in_member = (FILENAME != "Cargo.toml" && $0 ~ /^\[(.*\.)?(dev-|build-)?dependencies\]$/)
            # Table form, [dependencies.foo]: count it whatever its keys say.
            if (FILENAME != "Cargo.toml" && match($0, /dependencies\.[^]]+\]$/))
                print substr($0, RSTART + 13, RLENGTH - 14)
            next
        }
        /^[A-Za-z0-9_-]+(\.workspace)?[[:space:]]*=/ {
            name = $0; sub(/[.[:space:]=].*/, "", name)
            if (in_ws || (in_member && $0 !~ /workspace[[:space:]]*=[[:space:]]*true/)) print name
        }
    ' Cargo.toml crates/*/Cargo.toml gui/src-tauri/Cargo.toml | grep -v '^dovetail' | sort -u)
missing=$(comm -23 <(echo "$declared") <(echo "$plan_deps"))
if [[ -n $missing ]]; then
    echo "check.sh: dependencies with no stated reason; add a row to the table in docs/implementation-plan.md:" >&2
    echo "$missing" >&2
    exit 1
fi

# The same for the GUI frontend: every package in gui/package.json needs a row in the npm table.
plan_npm=$(awk -F'|' '
        /^\| Package \|/ { in_table = 1; next }
        in_table && !/^\|/ { exit }
        in_table { print $2 }
    ' docs/implementation-plan.md | grep -o '`[^`]*`' | tr -d '`' | sort -u)
declared_npm=$(node -e '
    const p = require("./gui/package.json");
    for (const k of Object.keys({ ...p.dependencies, ...p.devDependencies })) console.log(k);
' | sort -u)
missing=$(comm -23 <(echo "$declared_npm") <(echo "$plan_npm"))
if [[ -n $missing ]]; then
    echo "check.sh: GUI packages with no stated reason; add a row to the npm table in docs/implementation-plan.md:" >&2
    echo "$missing" >&2
    exit 1
fi

echo "check.sh: all checks passed"
