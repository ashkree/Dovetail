# Shelved from `main` on 2026-11-21 (task #52)

The TypeScript page and the TS generation, moved off `main` by user decision (during #47). They return
with `dovetail serve`, the reader and canvas page.

Source: `main` at commit `d87147f` (`git show d87147f:<path>`).

Here, with their paths kept:
- `page/`: the frontend;
- `crates/dovetail-api/tests/ts_export.rs`: writes the ts-rs declarations into `page/src/core/generated`;
- `crates/dovetail-api/tests/apply_cases.rs` and `tests/fixtures/apply/`: the apply cases and the JSON fixtures the TS reducer replays.

Not here, because they are edits, not files; they live in `main`'s history at `d87147f`:
- the `ts` feature, the `ts-rs` dependency and the `[[test]] ts_export` entry in `crates/dovetail-api/Cargo.toml`;
- the `#[cfg_attr(feature = "ts", ...)]` attributes in `crates/dovetail-api/src` (about 164);
- the ts_export step in `scripts/check.sh`.

Restore: copy the files out of `d87147f` (`git show d87147f:<path>`), re-add the attributes to the current
types, then re-add the `ts` feature, `ts-rs` in `[workspace.dependencies]` and the check.sh step. Do not
`git checkout d87147f -- crates/dovetail-api`: it would revert later dovetail-api work.
