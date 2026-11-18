#!/usr/bin/env bash
# Copies three real transcripts (cp only; nothing is written under ~/.claude) into a private temp
# dir, trims them with jq into fixtures/*.jsonl (git-ignored), and deletes the raw copies.
# Trimmed line: {"role":"user|assistant|tool","text":"..."}: text blocks, one line per tool_use.
set -euo pipefail
cd "$(dirname "$0")"
src="$HOME/.claude/projects/-home-mave-Documents-projects-Dovetail"
declare -A files=(
  [v2-rework]=d8515de8-3457-4e44-9884-b037996bb1a6
  [tui-return]=36131e81-c839-48e4-bb7e-cac576c21c6f
  [skills-grill]=ecef0f02-3f4c-4d59-8051-50483f8e5ae3
)
mkdir -p fixtures
raw=$(mktemp -d)
trap 'rm -rf "$raw"' EXIT
for name in "${!files[@]}"; do
  cp "$src/${files[$name]}.jsonl" "$raw/$name.jsonl"
  jq -c '
    select(.type=="user" or .type=="assistant")
    | .type as $role
    | (.message.content | if type=="string" then [{type:"text",text:.}] else . end)[]
    | if .type=="text" then {role:$role, text:.text}
      elif .type=="tool_use" then {role:"tool", text:(.name + " " + ((.input.file_path // .input.command // .input.pattern // .input.description // "") | tostring | .[0:100]))}
      else empty end
    | select(.text | length > 0)
    | select(.text | startswith("<") | not)' "$raw/$name.jsonl" > "fixtures/$name.jsonl"
  echo "fixtures/$name.jsonl: $(wc -l < "fixtures/$name.jsonl") lines, $(du -h "fixtures/$name.jsonl" | cut -f1)"
done
# Optional plain-markdown fixture, already in the repo.
md=../../docs/research/11-research-where-is-the-codebase-against-the-imple/report.md
if [[ -f $md ]]; then
  jq -Rsc '{role:"assistant",text:.}' "$md" > fixtures/report.jsonl
  echo "fixtures/report.jsonl"
fi
