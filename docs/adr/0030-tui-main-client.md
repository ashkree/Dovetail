# The TUI is the main client again; canvases and long reading move to a page the core serves

**Status: Accepted.** Drafted and grilled with the user, and approved by them, in the session of 2026-11-18.

Supersedes, on the client:
- `docs/design-v2.md` §1, "TUI v1 is frozen as built";
- ADR 0020, the GUI as main client and the TUI as a remote kit;
- ADR 0023, the Tauri shell hosting the core;
- ADR 0026, slice previews and declared variants.

Refines Decision 16 (readability). Settles the open question on buckets (Decision 11, `GLOSSARY.md` "Bucket").

The core, the driver, the API, storage, MCP and the v2 model are unchanged. The v2 model covers efforts, criteria, tasks, multi-repo projects, accounts, canvases and tool groups (ADRs 0021, 0022, 0024, 0025, 0027–0029).

Why:
- **The GUI was a step back for daily use.** The user uses the TUI every day at work and finds it more comfortable: keyboard flow, fast startup, no mouse as the main driver, and a fraction of the memory.
- **The GUI's own reasons have weakened.**
  - The user's design-iteration skill does in-app previews better than Dovetail would.
  - Canvases are still wanted, but they don't need a whole app window.
- **What the TUI really lacks is readability.** Tables, code and diagrams are hard to read, and one font size with tight spacing makes long text tiring. This is fixable in a terminal far enough for daily use. The rest can go to a page in the browser.

## Decision

**1. The TUI is the main client, and it is no longer frozen.**
- New features are built in the TUI.
- Polishing the TUI comes first.
- Dovetail is built inside the TUI from now on (the self-hosting rules in `AGENTS.md` apply).

**2. The GUI is shelved on a branch.**
- Before it leaves, its canvas code (about 190 lines of React Flow) is copied to where the page will live.
- Then `gui/` moves to a `gui-shelved` branch. It is deleted from `main` and taken out of `scripts/check.sh` (the npm, tsc, vitest and vite steps and the Tauri port check).
- A GUI may return later as one more client of the core. Nothing here blocks that.

**3. v2's model comes to the TUI.**
- The tree is project → efforts → tasks → agents.
- The Now / Next / Later **buckets are dropped**. Existing tasks move to the Unsorted effort.
- There is one project chat with effort tags and a filter for one effort's thread.
- Efforts get a screen of their own. Its layout comes from the design session in point 7.

**4. The core serves one local web page: the reader, then canvases.**
- One small HTTP server, bound to `127.0.0.1` on a free port, serves a single-page app.
- **Reader.** The escape hatch for anything a terminal draws badly. It shows the focused conversation, report, block or document with real typography: proportional fonts, true heading sizes, line spacing, rendered tables, highlighted code and rendered diagrams.
  - It follows the TUI's focus.
  - One key in the TUI opens it, or brings an open tab up to date.
  - The user still reads chat in the TUI.
- **Canvas.** ADR 0025 and 0027 as decided, in the same page. Built after the reader.
- The page is a client like the TUI: it receives events and sends `Command`s through the API types (the ts-rs generated types stay). It is keyboard-first; the mouse is welcome on the canvas but never the only way.
- Until the daemon lands, the TUI's in-process core serves the page.

**5. Readability in the terminal (refines Decision 16).** The target terminal is **kitty**, at work and at home.
- **Structure.**
  - A centred reading column with a maximum width and margins.
  - Blank rows between blocks.
  - Headings set off with weight, colour and rules.
  - Decision 16's solid-background toggle extends to content panes.
- **Heading sizes.** In kitty, headings are drawn larger with the text-sizing protocol. Elsewhere, and under tmux if the protocol doesn't pass through, they fall back quietly to normal size. Readability must not depend on this.
- **Wide blocks never wrap.**
  - Tables, code blocks and diagrams keep their lines and scroll sideways when focused.
  - Tables are box-drawn, with columns fitted to the space.
  - Code blocks have their own tint and syntax highlighting.
- **Mouse.** The wheel scrolls the pane under the pointer, including while the user is typing. The mouse does nothing else.

**6. The core runs on its own as `dovetail serve`, last.** This is ADR 0020's decided process model.
- Clients attach over the project socket.
- It is what makes SSH and tmux use possible later: the TUI attaches to a core on the home PC.
- Today the socket only carries `dovetail add` and MCP tool calls. Attaching needs:
  - commands and a stream of updates sent to several clients at once;
  - a snapshot when a client attaches;
  - a core that keeps running after the last client closes;
  - restart and project switching without relaunching the process.
- Whether sessions keep running after the TUI closes is decided when this step is planned. Until then, closing the TUI stops them, as today.

**7. Order of work.**
1. **A readability spike and a TUI design session, in parallel.**
   - The spike covers point 5, on real transcripts, in two or three variants for the user to choose between.
   - The design session covers the user's own declutter audit, the main layout, the effort screen and Home.
   - No screen code until the design session is done.
   - Its result goes into the TUI spec (`docs/ui-session-brief.md`, `docs/screens/ui/`).
2. **v2 views in the TUI** (point 3).
3. **The page:** reader, then canvas (point 4).
4. **The daemon** (point 6).

**8. `dvtl` is a short name for `dovetail`.**
- `scripts/bootstrap.sh` and `install.sh` install a `dvtl` symlink, and every subcommand works under both names.
- The binary, the launch flags sessions use (`dovetail mcp`, `dovetail hook`) and the state and config paths keep the name `dovetail`.

**9. Previews are dropped entirely.** ADR 0026 is withdrawn: no preview viewer, no declared variants, no preview command in the recipe. The user's design-iteration skill owns previews.

## Alternatives rejected

- **Keep the GUI and fix what grates.** The problems are the medium: the mouse as the default, startup time, memory, and working outside the terminal. Fixing them in Tauri gives a worse terminal.
- **TUI as driver plus a Tauri companion window for canvases.** A second native app to build, ship and keep in step. A browser page gives the same canvas with nothing to install, and works over SSH with a forwarded port.
- **No reader page; the terminal only.** Kitty can't give proportional type or rendered diagrams. Long reports and diagrams need somewhere better to go.
- **Canvases drawn in the terminal with kitty graphics.** The interaction is poor (no drag, no free placement), and graphics are awkward inside tmux. Kitty graphics stay for maps (Decision 23).
- **Real typography in the terminal by rendering markdown to images.** The text can't be selected or searched, has to be re-rendered on every resize, and breaks in tmux.
- **The daemon first.** The TUI already hosts the core and works. Readability and v2 views reach the user sooner, and the in-process core can serve the page.
- **Renaming the binary to `dvtl`.** Every session's launch flags, the bootstrap checks and the docs would change, and sessions started with the old flags would break on upgrade. A symlink gives the short name for free.

## Consequences

- **Spikes before the code that depends on them:**
  - `kitty-text-sizing`: heading sizes in kitty, including inside tmux.
  - `tui-readability`: point 5 on real transcripts.
  - `serve-page-memory`: the memory cost of the HTTP server in the core.
- **Dependencies, each needing a row in the plan's table:**
  - a small HTTP server crate in the core;
  - a syntax highlighter for the TUI (lean, chosen in the plan);
  - the page's frontend dependencies, carried over from `gui/`.
- **The memory target returns to the TUI's:** about 100 MB RSS. The browser tab is the user's own and is not budgeted. The 500 MB GUI budget is withdrawn.
- **Docs to update once this is accepted:**
  - `AGENTS.md`: what this is, scope, the hard constraints on the TUI, the crate list and the self-hosting note;
  - `docs/design-v2.md` §1, §7 and the order of work;
  - `docs/gui-layout-and-keys.md`: marked shelved;
  - `docs/architecture.md`;
  - `README.md`;
  - `GLOSSARY.md`: "Bucket" removed, and the reader and canvas page terms once their visible names are agreed in the design session.
- **Home is provisional.** The working brief is an alpha-nvim-style start screen in the TUI, with projects, their counts and a few actions. `dvtl` opens it; `dvtl <project>` goes straight into a project; the switcher stays for quick jumps inside a project. The design session settles it.
- **Effort proposals and criteria.** Proposals still arrive in the chat. Criterion claims show in the inbox and on the effort screen.
- **`:` commands come first** for creating projects, attaching repos and setting the account. Screens only if the commands feel clumsy.
