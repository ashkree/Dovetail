# Live model and effort switch

**Status:** run 2026-11-12 against Claude Code 2.1.322, in a scratch repo under `/tmp` (task #39). Both model and effort change live on a running headless session; no restart is needed.

Scripts: `spikes/live-model-effort/control.py` (every request once, then one tiny prompt) and `spikes/live-model-effort/effort.py` (effort changes read back with `get_settings`, no prompt). Each needs `mkdir -p /tmp/dt-model-spike && git -C /tmp/dt-model-spike init` first, then `python3 -I <script>`.

## Result

| Request | Answer | Takes effect |
| --- | --- | --- |
| `{"subtype":"set_model","model":"sonnet"}` | `success`, `response: {"model":"claude-sonnet-5-5"}` (the resolved name) | Yes: the next turn's `assistant.message.model` is `claude-sonnet-5-5` |
| `set_model` with an unknown name | `error`, `error: "Model 'bogus-model-xyz' not found"`, `error_code: "not_found"`, `model:` the unchanged current model | Old model kept |
| `{"subtype":"apply_flag_settings","settings":{"effortLevel":"high"}}` | `success`, no payload | Yes: `get_settings` → `applied.effort` is `"high"` |
| `apply_flag_settings` with an unknown effort (`"bogus"`) | **`success` anyway** | No: `applied.effort` stays at the old level |
| `set_model` to a model without effort support (`claude-haiku-4-5-20251001`) | `success` | `applied.effort` becomes `null` |
| `{"subtype":"get_settings"}` | `response.applied: {"model": "...", "effort": "..."}`, plus `effective` and `sources` | Read-only, no model call |
| `{"subtype":"list_models"}` | `response.models[]`: `value` (`opus`, `sonnet`, `haiku`, `fable`, full names), `resolvedModel`, `displayName`, `supportsEffort`, `supportedEffortLevels` | Read-only, no model call |

Effort levels accepted by `--effort` and `effortLevel`: `low`, `medium`, `high`, `xhigh`, `max`. On this account `opus`, `sonnet`, `haiku` and `fable` all list all five; older models (Haiku 4.5) have no `supportsEffort` at all.

Control responses do not come back in request order (`apply_flag_settings` answered before an earlier `set_model`), so a switch must chain its steps on the answers, not on write order.

## What Dovetail does with it

- The driver's `Session::set_model` sends `set_model` (only when the model changes: the core passes no model when the session already runs the one picked), then on success `apply_flag_settings {effortLevel}` (if an effort was picked), then `get_settings`, each step after the previous answer. The `applied` model and effort from `get_settings` are the confirmed state (`DriverEvent::ModelSettings`); a `set_model` error ends the chain (`DriverEvent::ModelSwitchFailed`) and nothing else is sent. Both events carry the switch's id, and the core takes only the answer to its latest switch; a second switch is refused while one is unanswered.
- Because a bad effort is accepted silently, the core compares the effort it asked for with `applied.effort` and reports a mismatch as an error.
- At start, once `initialize` has been answered, the driver also sends `get_settings` (to show the launch effort before any switch) and `list_models` (so the selector can gray out levels a model does not list). That read is not a switch: it reports with no switch id, is ignored if a switch was sent meanwhile, and a failure of it is only logged.
- The chosen model and effort are stored on the session row and passed as `--model`/`--effort` on the next start or resume.
