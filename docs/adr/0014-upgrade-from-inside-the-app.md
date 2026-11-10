# Upgrade from inside the app

Refines M10 decision 3 ("`scripts/install.sh` refuses while a Dovetail app runs") and plan risk R5. Approved by the user in the session of 2026-11-10.

Upgrading meant quitting Dovetail, running `scripts/install.sh`, waiting for the release build and opening it again. The refusal itself is right: running sessions start `dovetail mcp` and `dovetail hook` from the installed path, so replacing the binary under them makes new helpers talk to an older app. But restart and resume already work (M10), so the app can do the waiting and the restart for the user.

**Decision:** `:upgrade` in the app, and `dovetail upgrade` on the command line, build the new binary in the background into a staging folder under Dovetail's data directory, never over `~/.local/bin/dovetail`. When the build is done, an "Upgrade ready" item appears in Needs you. `y` restarts once every agent is idle, so no turn is cut off; `d` leaves the staged build for the next ordinary quit. The restart is the normal quit path, then the staged binary is renamed over the installed one, then the app `exec`s it in the same terminal with the same folder and arguments, and sessions resume as after any restart. The installed binary is only ever replaced while no app and no helper of it runs, so R5 still holds.

**Source:** `origin/main`, fetched into a throwaway worktree, never the user's working checkout, so a half-finished edit or a checked-out slice branch cannot reach the app the user works in. `:upgrade --local` builds the checkout instead, for the rare deliberate case.

**Considered options:**

- Replacing the binary while the app runs and leaving the old process alone: new hooks and MCP bridges would run a different version from the app, which is R5.
- Hot-patching Rust code in the running app: experimental, and not safe for the app the user works in.

**Consequences:**

- `dovetail upgrade` with no app running installs at once, as `scripts/install.sh` does now; with an app running it hands the staged build to it over the app socket. `install.sh` keeps refusing while an app runs.
- A failed build is an Error inbox item with the build's tail; nothing is replaced.
- Upgrade never overwrites the installed role prompts or recipe. When the new build's copies differ from the installed ones, the "Upgrade ready" item says so, and `scripts/bootstrap.sh --force` stays the way to take them.
- The build needs `cargo` and `git` on the machine running the app; bootstrap already checks both.
- Where an agent never goes idle, `y` waits; the item says which agents it is waiting for, and quitting with `:q` still swaps the staged build in once the app has shut down, so the next open runs it.
