---
created: 2026-07-07T08:21:25+12:00
title: e2e fixtures lock semantics, not just behavior
tags: [testing, duckdb, rust, design]
source: /ws done
---

## D04 e2e fixtures as semantics locks

- The e2e snapshot shows why the "count = *distinct* orphans" decision
  matters at the UI level: `trades` has one bad row (`cat_id = 99`) so the
  message reads "has 1 orphaned value" — a per-row count would drift
  whenever the same orphan repeats, making the number noise rather than
  signal.
- The clean fixture is doing double duty as a semantics lock: row 3's
  `cat_id = NULL` would make a naive `NOT IN`-style check (or a MATCH FULL
  interpretation) fail the run. Exit 0 here pins MATCH SIMPLE end to end,
  through the real binary and real DuckDB — not just at the unit seam.
- Note the snapshot's elision (`7 | - name: trades` … `18 | - name: cat_id`):
  the diagnostic renderer anchors D04 at the `foreign_key` constraint span
  but keeps the owning table line for context — that's `foreign_key_span`
  from phase 1 paying off in the CLI output.
