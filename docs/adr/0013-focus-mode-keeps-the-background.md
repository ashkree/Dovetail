# Focus mode keeps the background as it is

Refines Decision 16c ("a toggle gives the reading column a solid dark background, switched on automatically in focus mode") and plan item A1. Focus mode (`<leader>z`) no longer turns the solid background on: it only hides the side column and gives the conversation the full width. The background stays transparent unless the user turns it on with `<leader>B`, in focus mode or not.

Why: the user tried it on 2026-11-10 and did not like the solid background; they want focus mode to look like the usual layout.

Numbered 0013 because 0012 is taken by the end-of-day handoff work in flight on another branch.

Approved by the user in the session of 2026-11-10.

**Amendment, 2026-11-11:** the `<leader>B` toggle named above no longer exists. It was removed together with `:solid` and `Action::ToggleSolidBackground` (plan, "Small fixes decided 2026-11-11"), so there is no key that turns the solid background on. Focus mode and the conversation never paint a background; reading views were solid at first and later went transparent too (ADR 0018 amendment; `docs/ui-session-brief.md` has the current rule).
