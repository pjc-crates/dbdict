---
name: regular-code-reviews
description: run an independent code review at every /ws done phase boundary — user mandate 2026-07-07
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 84ccf91c-2176-4410-a016-ff96b5319687
---

Run `/code-review` (high effort) as part of every `/ws done` verify step, not
just tests/clippy/fmt. Mandated by the user 2026-07-07 after a review gap:
the last 3-agent review was 2026-07-06 (duckdb-spec phase 4, commit 05941b0)
and 23 commits / ~10.9k lines accumulated unreviewed before a catch-up review
was run.

**Why:** "even with solid TDD, independent code reviews are invaluable" (user,
verbatim). The 2026-07-05/06 reviews caught a verified correctness bug
(`resolve` contradicting `validate-meta` on shadowed typedef dependencies) —
exactly the cross-feature inconsistency class the test suite missed.

**How to apply:** at each phase boundary, review the phase's diff before or
alongside the phase commit; record findings and their resolution in impl.md
(the duckdb-spec session's `> 3-agent review` blockquote style is the
precedent). Related: [[ws-context-budget]] — if context is tight, the review
can run at the start of the next session against the phase commit instead.
