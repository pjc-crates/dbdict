---
created: 2026-07-08T15:16:36+12:00
title: shared orientation helper kills planner-validator drift
tags: [rust, duckdb, pattern, adversarial-review]
source: /state save
---

## extraction found live drift, and simplified both consumers

- The extraction did more than dedup: plan.rs was canonicalizing
  conjuncts with *exact* table-name comparison while rich.rs used
  case-insensitive `names_eq` — so a join spelled `"A.p = a.q"` could
  orient differently in the planner and the validator. That's precisely
  the silent-drift class finding 9 predicted; the shared
  `JoinExpr::oriented` now imposes the validator's (DuckDB-correct,
  case-folding) semantics on both.
- The plan.rs rewrite also became *simpler*: instead of tracking
  `one_side_is_left` with its own inverted cardinality mapping, it
  reuses `probe_left_directions()` and takes "the one side" as the
  non-probed side of `sides()` — one orientation vocabulary everywhere.
