---
created: 2026-07-07T10:11:40+12:00
title: paused before D05 phase 2 — CLI e2e + docs
tags: [rust, duckdb, workflow, testing]
summary: Session 20260707-0901 paused at the phase 1/2 boundary (context budget). Phase 1 done and committed (2d2ec20) — D05 specced, orientation normalization, fifth seam method, correlated-count native impl, 290 tests green. Phase 2 (CLI e2e fixtures + README/spec.md docs) not started; small, fully specified in impl.md.
---

## Goal
Work session 20260707-0901-d05-cardinality-verification: D05 (error,
rich only) — the data violates a relationship's declared `cardinality`;
some row matches more than one row on a declared "one" side when the
join is evaluated. All join types measured directly (equality overlap
with S06+D02/D03 accepted; range joins get their only coverage). PAUSED
at the phase 1/2 boundary per the context-budget rule.

## Current State
- Branch `duckdb-source`, HEAD `2d2ec20`, clean tree, `.active` points
  at the session.
- Phase 1 DONE (2d2ec20): D05 specced in site/validation.md (direction
  table: many-to-one probes left, one-to-many probes right, one-to-one
  both independently, one problem each; zero matches never violate;
  NULL join columns pass; count = over-matched probe rows);
  `check_relationships_data` in rich.rs runs after the per-table loop —
  per-conjunct canonicalization (right-to-left conjuncts mirrored) then
  probe orientation (probing the right side mirrors ops again:
  Eq↔Eq, Ge↔Le, Gt↔Lt via `flip_op`); self-joins positional; under
  --table a relationship is in scope if it touches the selected table;
  skips mirror D04 (absent table → M06, absent column → M02); query
  failure → UnreadableSource at the join-text span. Fifth seam method
  `count_overmatched_rows(db_file, probe_table, other_table,
  &[OrientedConjunct])` — conjuncts cross as data (columns + JoinOp),
  seam doc updated with the fifth-method re-check. Native impl is a
  correlated count (`WHERE (SELECT count(*) FROM other o WHERE …) > 1`)
  with empty-conjunct guard; `ProblemKind::CardinalityViolation
  { count }` → D05, Level::Data, anchored at join_text + cardinality
  spans (S06 two-span pattern), message names probe side, other side,
  and declared cardinality. 290 tests / 0 failed (was 274: 7 fake-
  backend + 9 real-duckdb, red first); clippy + fmt clean. Insights
  saved (composed guarantees + orientation normalization, 20260707-0953).
- Phase 2 NOT STARTED.

## Key Decisions
(all annotated in goal.md / impl.md of the session dir)
- All joins measured directly, not just range joins — D02/D05
  double-report on duplicated equality-join pks accepted (user).
- Severity error, consistent with D01–D04 (user).
- Zero matches never violate — cardinality bounds multiplicity, not
  totality (user).
- one-to-one checks both directions independently, one problem each
  (user).
- Seam kept at the fifth method; conjuncts cross as data
  (OrientedConjunct), never SQL text — re-check at the sixth.

## Next Steps
Phase 2 (small, fully specified in impl.md — read it on resume; mirrors
the D04 session's phase 2, commits 2d2ec20/08f60db are the pattern):
1. crates/dbdict-cli/tests/cli.rs: seeded rich-data fixture gains a
   `periods` table with overlapping ranges + a many-to-one range
   relationship → snapshot shows D01–D05 (rename test to
   `..._d01_through_d05`; delete old snapshot with the rename; review
   .snap.new before accepting via mv). Clean fixture gains the same
   shape with non-overlapping ranges → still exits 0.
   NOTE: the seeded fixture already has `trades.cat_id = categories.id`
   many-to-one — adding a second relationship is additive; keep S06
   satisfied (mark `periods.start` unique, as the core fixtures do) so
   the snapshot stays D-level only.
2. README.md validate-data bullet: add D05 alongside D01–D04.
3. site/spec.md: the relationships section's `cardinality` bullet points
   at D05 in validation.md (the D03/D04 cross-reference pattern).
4. Verify: cargo test --workspace; clippy; fmt --all --check (run from
   repo root); seeded fixture exits 1 with D01–D05, clean exits 0.
   Then /ws done and /ws close.

## Relevant Files
- .claude-work/sessions/20260707-0901-d05-cardinality-verification/{goal,impl}.md
- crates/dbdict/src/rich.rs — check_relationships_data, OrientedConjunct,
  flip_op, cardinality_str, db_column_in
- crates/dbdict/src/problem.rs — CardinalityViolation variant
- crates/dbdict-duckdb/src/native.rs — count_overmatched_rows, op_sql
- crates/dbdict/tests/rich_data.rs — D05 section (~line 1170+)
- crates/dbdict-duckdb/tests/data_queries.rs — D05 section (~line 290+)
- crates/dbdict-cli/tests/cli.rs — the two fixtures to extend (~line 320+)
- site/validation.md (D05 spec), site/spec.md (~line 382 cardinality
  bullet), README.md (~line 76)
