# Behaviour changed on purpose

The log Rule 0 requires. [PLAN.md](../PLAN.md) holds the rules and the work
still open; this file holds what has already changed and why, so a decision can
be found later without reading the plan around it.

Rule 0 licenses changing behaviour, not changing it silently. One entry per
change, newest first, and each says what it was, what it is, why that side, and
how to check it without the app:

- **Before / After** — the behaviour on each side, concretely enough to tell
  whether you are looking at the old one.
- **Why** — which rule this serves. "Rule 0" alone is not a reason; name the
  inconsistency, the dead feature, or the second model being collapsed.
- **CLI check** — the `spectra` invocation that shows it, or an honest note
  that none applies and what covers it instead.
- **Verification** — the three suites at the time of the change.
