#!/usr/bin/env bash
# Which stream-json input events does agy accept? Each line is sent alone; an unsupported
# event is reported as "ignoring unsupported stream input message event".
cd /tmp/agy-spike/repo || exit 1
for event in control_request interrupt cancel control set_model user_message; do
  echo "== $event"
  echo "{\"event\":\"$event\"}" |
    timeout 60 agy --input-format stream-json --output-format stream-json \
      --model gemini-3.8-flash-low -p="" 2>&1 | grep -E 'warning|error' | head -2
done
