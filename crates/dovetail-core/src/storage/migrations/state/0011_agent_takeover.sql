-- Task #32 (M14, 2026-11-10): a session's take-over lock, as JSON (`takeover::TakeoverLock`:
-- since when, the native process's pid and start time, where the transcript stood), or NULL
-- when Dovetail drives it headless. Stored before the headless process is stopped, so a crash
-- while the user works in native `claude` leaves the lock in place for the next open.
ALTER TABLE agents ADD COLUMN takeover TEXT;
