# Plan: the viewer on real slice previews, with declared variants (ADR 0026)

Spec: `docs/adr/0026-slice-previews-and-declared-variants.md`, `docs/design-v2.md` section 7, `docs/gui-layout-and-keys.md`. Model for the shape of the work: canvases (`docs/plan/canvas-core.md`).

## 1. Preview process (core)
- `recipe.rs`: `preview: Option<String>` at the top level and in `RepoRecipe`; `for_repo` applies the override.
- API `preview.rs`: `PreviewState`, `Preview { agent, state }`; `Snapshot.previews`; `Command::{StartPreview, RestartPreview}`; `Event::PreviewChanged`; apply drops a removed agent's preview.
- Core `preview.rs`: `Core.previews` (in memory), one task per run owning the child (own process group, optional capped `systemd-run --scope` unit `<session unit>-preview`), output tail, readiness check, exit report as `Input::PreviewReady` / `Input::PreviewExited` with a run number so a late report of an old run is ignored. Stopped by `close_slice` and `begin_quit`.
- Tests: a trivial static server (python `http.server`) on scratch data: start, running with URL, fetch the page; no recipe key; failing command shows its output; restart gives a new port-free run; stop on archive and on quit.

## 2. Variants
- Migration `0020_design_variants.sql`: `design_variants (agent_id PRIMARY KEY, slots, selection)`; removed with the slice.
- API `design.rs`: `VariantLevel`, `VariantSlot`, `VariantSet`; `Snapshot.variants`; `Command::{SelectVariant, KeepVariants}`; `Event::VariantsUpserted`. Tool `design_declare_variants` in suite `design` (slice default suites), validated whole.
- Core: DB methods, handlers, the keep prompt. Shared apply cases `tests/fixtures/apply/design.json`; TypeScript regenerated.

## 3. Slice role snippet
Three lines in `config/roles/slice.md`; `scripts/bootstrap.sh` untouched (it already installs role files without overwriting).

## 4. GUI
- View adapter builds `variants` from the snapshot (slot name as id, indices for the existing panel) and the preview state; `translate.ts` maps `start_preview`, `select_variant`, `keep_variants`; `pending.ts` loses variants.
- Viewer: iframe on the running URL; states for not started, starting, failed (output and restart), no command (how to add the line); `R` reloads, `gx` opens the URL through the existing open-external command; the page gets `{ dovetailVariants }`. `MockCore` keeps the sample page. Tauri CSP already allows local frames; checked.

## 5. Verify
`scripts/check.sh`; then scratch repo + fake driver run, described in the PR.
