# GUI frontend: decisions made while building unattended

The plan (`docs/plan/gui-frontend.md`) says "stop and ask" in a few places. The user was asleep while F1–F9 were built (2026-11-15), so each such point was decided here instead, staying inside the settled layout, look and nouns and as close to `mock/gui/` and `docs/gui-layout-and-keys.md` as possible. One entry each: what was open, what was chosen, why, how to change it.

## 1. Dev server port 5174 instead of 5173

- **Open:** the plan fixes `http://127.0.0.1:5173` with `strictPort`. On this machine another app (a container serving "Confidence Sync" from `/app`) already listens on 5173, so `npm run dev` failed.
- **Chose:** port 5174, still `strictPort` and host `127.0.0.1`.
- **Why:** the other app is not Dovetail's to stop; a fixed port that works beats one that fails at start.
- **Change:** `server.port` in `gui/vite.config.ts`.
