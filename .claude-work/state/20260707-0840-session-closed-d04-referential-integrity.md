---
created: 2026-07-07T08:40:18+12:00
title: session closed — D04 referential integrity
tags: [rust, duckdb, testing, workflow]
summary: Session 20260707-0735 closed with both phases done. D04 (orphaned foreign-key values) fully landed — spec, S01 tightening, shared fk-target resolution, null-safe NOT EXISTS anti-join, CLI e2e + docs. 274 tests green; HEAD 08f60db.
---

## Goal
Work session 20260707-0735-d04-referential-integrity, now CLOSED. D04
(error, rich only): orphaned non-NULL values in `foreign_key` columns,
checked against the `primary_key` column each relationship's *equality*
conjunct pairs them with. Read-only queries, no constraints installed.

## Current State
- Branch `duckdb-source`, HEAD `08f60db`, all phases complete.
- Phase 1 (98c81f5): D04 specced in site/validation.md; S01 tightened
  to equality conjuncts; shared `DataDict::foreign_key_targets`
  resolution used by S01 and D04; fourth seam method
  `DuckdbBackend::count_orphaned_values`; native impl is a null-safe
  `NOT EXISTS` anti-join (NOT IN gotcha locked by test);
  `ProblemKind::OrphanedValues` anchored at the `foreign_key`
  constraint span; absent pk-side table/column skipped (M06/M02 cover).
- Phase 2 (08f60db): seeded CLI e2e fixture shows D01–D04 (test
  `validate_data_rich_reports_d01_through_d04`); clean fixture carries
  a NULL fk and exits 0 (MATCH SIMPLE locked end to end); README and
  site/spec.md `foreign_key` bullet cross-reference D04.
- 274 workspace tests green; clippy + fmt clean.
- summary.md written; session dir complete (goal, impl, summary).

## Key Decisions
- NULLs excluded from D04 (SQL MATCH SIMPLE); nulls in `required`
  columns remain D01's business.
- Every declared fk→pk equality pair checked independently, one
  problem per violating pair.
- S01 tightened to equality conjuncts so S01 and D04 resolve
  identically (shared resolution makes drift impossible).
- Narrow-method seam kept at the fourth (first cross-table) method —
  cross-table changed the SQL inside the method, not the seam's shape.
- Out of scope, possible future sessions: composite-fk tuple semantics
  (format cannot declare one), cardinality-vs-data verification.

## Next Steps
- No open work in this session. Candidate next sessions:
  - cardinality verification against data (one-to-one / many-to-one) —
    flagged in goal.md scope as a plausible future check
  - whatever the data-check queue holds next (D-level checks so far:
    D01 nulls, D02 dup pks, D03 dup uniques, D04 orphans)
- Start any new work with `/ws new`.

## Relevant Files
- .claude-work/sessions/20260707-0735-d04-referential-integrity/{goal,impl,summary}.md
- crates/dbdict/src/model.rs — foreign_key_targets, FkTarget
- crates/dbdict/src/rich.rs — check_table_data D04 loop, seam trait
- crates/dbdict/src/problem.rs — OrphanedValues variant
- crates/dbdict-duckdb/src/native.rs — count_orphaned_values
- crates/dbdict-cli/tests/cli.rs — rich-data e2e fixtures (D01–D04)
- site/validation.md (D04 + S01 spec), site/spec.md, README.md
