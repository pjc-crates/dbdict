---
created: 2026-07-07T10:20:34+12:00
title: one fixture, three cardinality outcomes
tags: [rust, duckdb, testing]
source: /ws done
---

## fixture dates cover all three cardinality outcomes at once
- The seeded fixture gets three trade dates chosen so each cardinality outcome appears once: one date in the *overlap* of two periods (over-match → D05), one inside a single period (exact match → pass), one matching no period (zero matches → pass, locking "cardinality bounds multiplicity, not totality" end to end).
- `end` is a reserved word in DuckDB, so the raw `CREATE TABLE` needs `"end"` quoted — but the YAML side doesn't, because the join parser treats it as a plain identifier and the backend quotes every identifier it renders (`quote_ident`).

## adjacent spans collapse; one constraint, two jobs
- The diagnostic renderer collapses D05's two anchor spans (join text + cardinality) into one block because they're adjacent lines — the join line renders as context with the caret under `many-to-one`. Same mechanism as S06, but here it reads as "this declaration, on this join, is what the data contradicts."
- `periods.start` marked `unique` did double duty: it satisfies S06's permissive range rule (keeping the snapshot D-level only) *and* gets D03 coverage on a second table for free — the fixture's period starts are distinct, so it passes silently.
