# MCP menu actions on a headless session

**Status:** run 2026-11-21 against Claude Code 2.1.325, in a scratch repo under `/tmp` (task #44). Every action the `/mcp` menu has, except one, is a control request on the stream-json channel the driver already uses. `/mcp` sent as a user message is not usable.

Scripts: `spikes/mcp-headless/probe.py` (reconnect, toggle, authenticate, status, `/mcp` as a user message) and `probe2.py` (tool details, `mcp_set_servers`, the OAuth start for a claude.ai connector). Each needs `mkdir -p /tmp/dt-mcp-spike && git -C /tmp/dt-mcp-spike init` first, then `python3 -I spikes/mcp-headless/<script>`. They start `claude` with the driver's flags (`-p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --permission-prompt-tool stdio --replay-user-messages --session-id … --model haiku --mcp-config …`) and three test servers: `good` (the toy stdio server from `spikes/mcp/`), `broken` (a command that does not exist) and `needsauth` (a local HTTP server that answers 401). Only `probe.py`'s last step makes a model call (none was made: see below). The user's real MCP config was not touched; the claude.ai connectors the account has came along and showed up in the answers.

## Recommendation

The menu offers these, each one control request (`{"type":"control_request","request_id":…,"request":{…}}`), answered by a `control_response` that `success` or `error`s:

| Menu action | Request | Notes |
| --- | --- | --- |
| **List servers, status, tools, config** (the left and right panes) | `{"subtype":"mcp_status"}` | One answer holds everything; see below. Send on open and again after every action. |
| **Reconnect** | `{"subtype":"mcp_reconnect","serverName":N}` | Success is an empty answer. A failing server answers `error` with the reason, so the menu can show it at once. |
| **Disable / enable** | `{"subtype":"mcp_toggle","serverName":N,"enabled":false\|true}` | Takes effect at once; enabling reconnects. |
| **Authenticate** (needs-auth servers) | `{"subtype":"mcp_authenticate","serverName":N,"redirectUri":U,"redirectUriOnly":true}` | Returns `authUrl` for Dovetail to open or show; finish with `mcp_oauth_callback_url`. Only HTTP/SSE/claude.ai servers; see caveats. |
| **Clear authentication** | `{"subtype":"mcp_clear_auth","serverName":N}` | Success with `{}`. |

Do **not** build on: `/mcp` as a user message (a canned one-line summary, no menu); `mcp_message` (accepted, returns no payload); `mcp_read_resource` (only `ui://` URIs); `mcp_set_servers` (replaces the server set: it would drop the `--mcp-config` servers Dovetail launched with; leave it).

The status list is a **snapshot**: no event was seen for a status change in these runs (`system/init` was the only unsolicited list; a server dropping mid-session was not tested). Dovetail has to poll `mcp_status` while the menu is open and after each action. Tool **descriptions and schemas are not available** over control; the right pane gets tool names plus the `readOnly`/`destructive` annotations only.

Driver work this implies (for the build task, not done here): a `Session::mcp_status/reconnect/toggle/authenticate/clear_auth` family next to `set_model` in `switch.rs`, one `DriverEvent::McpStatus` carrying the parsed list, and the chain-on-answer rule from the live-model spike (answers do not come back in request order).

## Evidence

### `system/init` is thin, and late

Headless Claude Code writes `system/init` only when the first user message arrives, not after `initialize`. Its `mcp_servers` is `[{"name","status","source"}]` and nothing else:

```json
{"name": "good", "status": "connected", "source": "dynamic"}
{"name": "broken", "status": "failed", "source": "dynamic"}
{"name": "claude.ai Google Calendar", "status": "needs-auth", "source": "claudeai"}
```

Statuses seen: `connected`, `failed`, `needs-auth`, and (from `mcp_status`) `disabled`. `source` is `dynamic` for `--mcp-config` servers and `claudeai` for the account's connectors. So a menu opened on a session that has not yet taken a turn has nothing from `init`; `mcp_status` works straight after `initialize`.

### `mcp_status`: works, and richer than `init`

`response.mcpServers[]`, one entry per server:

```json
{"name":"good","status":"connected","serverInfo":{"name":"spike","version":"0"},
 "config":{"type":"stdio","command":"python3","args":["…/server.py"]},
 "transport":"stdio","scope":"dynamic","source":"dynamic",
 "tools":[{"name":"ping","annotations":{}},{"name":"who","annotations":{}}]}
{"name":"broken","status":"failed","error":"ENOENT: no such file or directory, posix_spawn 'stdio'",
 "config":{"type":"stdio","command":"/nonexistent/binary","args":[]},"transport":"stdio","scope":"dynamic","source":"dynamic"}
{"name":"needsauth","status":"failed","error":"Dynamic Client Registration rejected (HTTP 401): ",
 "config":{"type":"http","url":"http://127.0.0.1:18765/mcp"},"transport":"http",…}
{"name":"claude.ai Claude Docs","status":"connected","serverInfo":{…},
 "config":{"type":"claudeai-proxy","url":"…","id":"mcpsrv_…"},"transport":"claudeai-proxy",
 "sessionProxied":true,"scope":"claudeai","source":"claudeai",
 "tools":[{"name":"read","annotations":{"readOnly":true}},{"name":"delete","annotations":{"destructive":true}},…]}
```

So: status, `error` text for failures, `serverInfo`, transport config, scope, and the tool list with annotations. A `disabled` server has no `serverInfo` and no `tools`. Env values are not shown in `config` for the stdio server (the test server's `env` was absent), so secrets in env stay out of it.

### Reconnect: works

| Request | Answer |
| --- | --- |
| `mcp_reconnect` `good` | `success`, no payload; server stays `connected` |
| `mcp_reconnect` `broken` | `error`: `"ENOENT: no such file or directory, posix_spawn 'stdio'"` (the same text `mcp_status.error` carries); status stays `failed` |
| `mcp_reconnect` `nope` | `error`: `"Server not found: nope"` |

### Enable / disable: works

`mcp_toggle` `good` `enabled:false` → `success`; `mcp_status` then shows `good` as `"status":"disabled"`, with no tools. `enabled:true` → `success`; the next `mcp_status` shows it `connected` again with its tools. No restart.

### Authenticate: works for the right server types, with caveats

| Request | Answer |
| --- | --- |
| `mcp_authenticate` for the claude.ai connector that was `needs-auth`, `redirectUriOnly:true` | `success`, `response: {"authUrl":"https://claude.ai/api/organizations/…/mcp/start-auth/mcpsrv_…?product_surface=sdk-cli","requiresUserAction":true,"callbackExpected":false}`. Nothing opened a browser. |
| `mcp_authenticate` `needsauth` (plain HTTP server that only says 401) | `error`: `"SDK auth failed: Dynamic Client Registration rejected (HTTP 401):"`. That is the test server's fault (no OAuth metadata): a real OAuth server was not available to test the code-return path. |
| `mcp_authenticate` `good` (stdio) | `error`: `"Server type \"stdio\" does not support OAuth authentication"` |
| `mcp_oauth_callback_url` with no flow started | `error`: `"No active OAuth flow for server: needsauth"` |
| `mcp_clear_auth` `needsauth` | `success`, `response: {}` |

The authentication path is therefore: `mcp_authenticate` returns an `authUrl`; the user opens it (Dovetail shows it or runs `xdg-open`); a local-callback server finishes the flow itself (`callbackExpected`) or the pasted redirect URL goes back with `mcp_oauth_callback_url` (`{"subtype":"mcp_oauth_callback_url","serverName":N,"callbackUrl":URL}`). **Not verified end to end**: the claude.ai connector's `callbackExpected:false` means the browser finishes it on claude.ai's side, and the full round trip was not completed (it would authorise a real account connector). The build task should check that a `needs-auth` server turns `connected` after the user finishes, by polling `mcp_status`.

Oddity worth knowing: while the session reported `claude.ai Google Calendar` as `needs-auth`, `claude mcp list` (a separate process) reported it `✔ Connected`. The CLI health check and the session's cache disagree, so the menu must trust the session's `mcp_status`, not the CLI.

### Other probes

- **`/mcp` as a user message:** the session answers locally, no model call, `total_cost_usd: 0`: an `assistant` message (`model: "<synthetic>"`) with the text `"9 MCP server(s): 6 connected, 3 not connected, 0 disabled. Use \`/mcp\` in the terminal for details."`, then `result`. `/mcp` appears in `system/init` `slash_commands`. It is a count, not a menu.
- **`mcp_message`** (`server_name`, `message`: a JSON-RPC `tools/list`): `success` with no `mcp_response`; no tool descriptions came back. Not a way to read details.
- **`mcp_read_resource`:** `error`: `"mcp_read_resource: uri must use the ui:// scheme"`.
- **`mcp_set_servers`:** `success`, `{"added":["extra"],"removed":[],"errors":{"extra":"Connection closed"}}` (the error is my test server missing its log directory). It takes the *whole* server map and drops servers not in it, so it is not a way to add one server.
- **`claude mcp` CLI:** `list`, `get`, `add`, `remove`, `login`, `logout` act on config files and a fresh process, not the running session. `add`/`remove` write the user's or project's config, which Dovetail must not do (nothing in user repos). Not used.

### Where the subtypes came from

None are documented in the stream protocol notes. They are the control requests of the Agent SDK's `Query` class, found in the installed binary: `mcp_status`, `mcp_reconnect` (`reconnectMcpServer`), `mcp_toggle` (`toggleMcpServer`), `mcp_authenticate`, `mcp_clear_auth`, `mcp_oauth_callback_url`, `mcp_set_servers`, `mcp_message`, `mcp_read_resource`. They may change between versions: the driver should treat an `error` for them as "unavailable" and log it, as it does for `get_settings`.

## Open points for the build task

- Whole OAuth round trip against a real OAuth MCP server (code returned via `mcp_oauth_callback_url` and via the local callback).
- Whether a server that is `failed` because it needs auth ever shows `needs-auth` for non-claude.ai HTTP servers (the 401-only test server showed `failed`).
- Poll interval for `mcp_status` while the menu is open (suggest on open, after each action, and every few seconds).

## Build check: reconnect and toggle on a claude.ai connector (task #44, 2026-11-21)

`spikes/mcp-headless/probe3.py` on `claude.ai Claude Docs` (a connected `claudeai-proxy` connector; no model call). Same behaviour as the local test servers:

| Request | Answer / result |
| --- | --- |
| `mcp_reconnect` | `success`, no payload; still `connected`, 8 tools |
| `mcp_toggle` `enabled:false` | `success`; `mcp_status` shows `disabled`, no `serverInfo`, no tools |
| `mcp_toggle` `enabled:true` | `success`; `connected` again with 8 tools |

The connector was re-enabled in a `finally` block and `mcp_status` confirmed `connected` at the end. Nothing was logged out or changed persistently. The toggle is per session: it is not written to claude.ai. `mcp_authenticate` / `mcp_clear_auth` on a connector were not tried (D28: the round trip is done with the user).
