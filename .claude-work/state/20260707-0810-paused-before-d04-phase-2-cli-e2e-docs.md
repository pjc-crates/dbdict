---
created: 2026-07-07T08:10:04+12:00
title: paused before D04 phase 2 — CLI e2e + docs
tags: [rust, duckdb, workflow, testing]
summary: Session 20260707-0735 paused at the phase 1/2 boundary (context budget). Phase 1 done and committed (98c81f5) — D04 specced, S01 tightened to equality, shared fk-target resolution, NOT EXISTS anti-join, 274 tests green. Phase 2 (CLI e2e fixtures + README/spec.md docs) not started; small, fully specified in impl.md.
---

## Goal
Work session 20260707-0735-d04-referential-integrity: D04 (error, rich
only) — orphaned values in `foreign_key` columns, checked against the
`primary_key` column each relationship's equality conjunct pairs them
with. PAUSED at the phase 1/2 boundary per the context-budget rule
(don't start a phase at ≥25%).

## Current State
- Branch `duckdb-source`, HEAD `98c81f5`, clean tree, `.active` points at
  the session.
- Phase 1 DONE (98c81f5): D04 specced in site/validation.md (NULL
  exclusion = MATCH SIMPLE; every declared fk→pk pair checked
  independently; count = distinct orphans); S01 tightened to equality
  conjuncts (fixture spec/s01-fk-range-conjunct-only.yaml) with its
  validation.md wording updated; shared resolution
  `DataDict::foreign_key_targets` (model.rs) used by both S01 and D04 so
  they can't drift; fourth seam method
  `DuckdbBackend::count_orphaned_values`; native impl is a null-safe
  `NOT EXISTS` anti-join (NOT IN would report zero orphans if the pk
  column holds a NULL — dedicated test locks this);
  `ProblemKind::OrphanedValues { count }` → D04, Level::Data, anchored at
  the `foreign_key` constraint span via new `foreign_key_span`, message
  names the pk target (dict spellings); `check_table_data` now takes the
  whole dict + full schema list; pk-side table absent → skip (M06
  reported), pk-side column absent → skip (M02 reported). 274 tests / 0
  failed (was 258); clippy + fmt clean. Insights saved (anti-join
  gotcha + asserting-fakes pattern, 20260707-0806).
- Phase 2 NOT STARTED.

## Key Decisions
(all annotated in goal.md / impl.md of the session dir)
- NULLs excluded from D04 (SQL MATCH SIMPLE) — user decision.
- Every declared fk→pk equality pair checked independently, one problem
  per violating pair — user decision.
- S01 tightened to equality conjuncts in this session so S01 and D04
  resolve identically — user decision.
- Narrow-method seam re-checked and kept at the fourth (first
  cross-table) method.

## Next Steps
Phase 2 (small, fully specified in impl.md — read it on resume; mirrors
the D03 session's phase 2, commits 151bc1b/90847c0 are the pattern):
1. crates/dbdict-cli/tests/cli.rs: seeded rich-data fixture gains a
   second table + fk column with an orphaned value → snapshot shows
   D01+D02+D03+D04 (rename test to match coverage; delete old snapshot
   with the rename; review .snap.new before accepting via mv). Clean
   fixture gains the same shape with all fk values present plus a NULL
   fk → still exits 0.
2. README.md validate-data bullet: add D04 alongside D01–D03.
3. site/spec.md: `foreign_key` constraint bullet (and/or relationships
   section) points at D04 in validation.md — the D03 cross-reference
   pattern (see the `unique` bullet done last session).
4. Verify: cargo test --workspace; clippy; fmt; seeded fixture exits 1
   with D01–D04, clean exits 0. Then /ws done and /ws close.

## Relevant Files
- .claude-work/sessions/20260707-0735-d04-referential-integrity/{goal,impl}.md
- crates/dbdict/src/model.rs — foreign_key_targets, FkTarget
- crates/dbdict/src/rich.rs — check_table_data D04 loop, foreign_key_span
- crates/dbdict/src/problem.rs — OrphanedValues variant
- crates/dbdict-duckdb/src/native.rs — count_orphaned_values (NOT EXISTS)
- crates/dbdict-cli/tests/cli.rs — the two fixtures to extend (~line 320+)
- site/validation.md (D04 spec + S01 wording), site/spec.md (~line 364
  constraints, ~line 374 relationships), README.md (~line 76)
