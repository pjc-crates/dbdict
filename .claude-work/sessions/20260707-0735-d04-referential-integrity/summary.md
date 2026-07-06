# summary: D04 referential integrity check

started: 2026-07-07 07:35
closed: 2026-07-07T08:39:45+12:00

## goal

Add D04 (error, rich format only): a `foreign_key` column contains a
non-NULL value that does not exist in the `primary_key` column it is
paired with by a relationship's *equality* conjunct. Read-only queries
and diagnostics only — no constraints installed in the database (the
types-only DDL decision stands). Tighten S01 to equality conjuncts in
the same session so S01 and D04 resolve fk→pk pairings identically.

## what was accomplished

### phase 1: spec + S01 tightening + core check through the seam (98c81f5)
- D04 specced in site/validation.md before implementation (D02/D03
  convention): NULL exclusion = SQL MATCH SIMPLE; every declared fk→pk
  equality pair checked independently, one problem per violating pair;
  count = distinct orphaned values; self-joins work naturally
- S01 tightened to equality conjuncts — a range conjunct (`>=`, `<`, …)
  no longer satisfies it (red-first fixture
  spec/s01-fk-range-conjunct-only.yaml); S01 wording updated
- shared resolution `DataDict::foreign_key_targets` (model.rs) used by
  both S01 and D04 — one resolution, the two checks cannot drift
- fourth narrow seam method `DuckdbBackend::count_orphaned_values`;
  native impl (dbdict-duckdb) is a null-safe `NOT EXISTS` anti-join —
  `NOT IN` would silently report zero orphans if the pk column holds a
  NULL; a dedicated real-duckdb test locks the gotcha
- `ProblemKind::OrphanedValues { count }` → D04, Level::Data, anchored
  at the column's `foreign_key` constraint span (new `foreign_key_span`);
  message names the pk target so multi-target columns stay tellable apart
- skips: pk-side table absent from db (M06 already reported) and pk-side
  column absent (M02 already reported) — both fake-backend tested
- 274 workspace tests green (was 258): S01 snapshot, 8 fake-backend,
  7 real-duckdb (distinct count, NULL-fk exclusion, NOT-IN lock,
  case-insensitive match, hostile-name quoting both tables, self-join)

### phase 2: CLI e2e + docs (08f60db)
- seeded rich-data CLI fixture gains a `categories` table and fk column
  with an orphaned value — snapshot shows D01+D02+D03+D04, D04 anchored
  at the `foreign_key` constraint (test renamed to
  `validate_data_rich_reports_d01_through_d04`)
- clean fixture carries all-present fk values plus a NULL fk and still
  exits 0 — locks MATCH SIMPLE NULL exclusion end to end through the
  real binary
- README validate-data bullet lists D04 alongside D01–D03; site/spec.md
  `foreign_key` bullet states the equality-conjunct pairing rule and
  cross-references D04 in validation.md

## key decisions

- NULLs excluded from D04 (SQL MATCH SIMPLE — NULL means "no
  reference"); nulls in `required` columns remain D01's business
- every declared fk→pk equality pair checked independently, one problem
  per violating pair — as in SQL, each FK constraint stands alone
- S01 tightened to equality conjuncts in this session so S01 and D04
  pair identically — a range predicate relates columns without
  referencing one from the other
- narrow-method seam re-checked and kept at the fourth (first
  cross-table) method: cross-table changes the SQL inside the method,
  not the seam's 1:1 check-to-query shape; fakes stay canned-count stubs
- out of scope: composite-fk tuple semantics (format cannot declare
  one), cardinality-vs-data verification (future session), legacy path
  (preserved, not extended)

## insights captured

- .claude-work/insights/20260707-0806-null-safe-anti-joins-and-asserting-fakes.md
  — the NOT IN null-masking gotcha; fakes that assert call shape
- .claude-work/insights/20260707-0821-e2e-fixtures-lock-semantics-not-just-behavior.md
  — e2e fixtures as semantics locks (distinct-orphan count, NULL fk
  exit-0, constraint-span anchoring visible in snapshots)
