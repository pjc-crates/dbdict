# summary: D03 unique column check

started: 2026-07-06 17:13
closed: 2026-07-07T07:25:40+12:00

## goal

Close the last unverified declared constraint: columns marked `unique` that
aren't the primary key were never checked against the data (D02 covers only
the `primary_key` column set). Under the declare-then-check model (types-only
DDL, 2026-07-06 decision), a `unique` declaration is worthless until
something queries for violations. Deliverable: **D03** (error, rich format
only) — a non-NULL value in an explicitly-`unique` column occurring in more
than one row, specced before implementation, TDD red-first.

## what was accomplished

### phase 1: spec + core check (commit 90847c0)
- D03 specced in site/validation.md before implementation (the D02
  convention), including the NULL-exclusion rationale and the D02 overlap
  rule.
- `DuckdbBackend` trait gained a third narrow method,
  `count_duplicate_values(db_file, table, column)`; native impl in
  dbdict-duckdb filters `WHERE col IS NOT NULL` before GROUP BY/HAVING.
- `check_table_data` runs D03 after D01/D02 for each explicitly-`unique`
  column, skipping a column that alone IS the primary-key set; problems
  anchor at the column's `unique` constraint span (new `uniqueness_span`).
- New `ProblemKind::DuplicateValues { count }` → code D03, Level::Data.
- D02's function-level early returns restructured into scoped `if`s — they
  would have silently starved D03 for pk-less tables.
- 10 new tests (6 fake-backend incl. sole-pk skip and composite-member
  check; 4 real-duckdb incl. the repeated-NULLs-not-duplicates lock).

### phase 2: CLI e2e + docs (commit 151bc1b)
- Seeded rich-data e2e fixture gained a `unique` column with a duplicate —
  snapshot shows D01+D02+D03 with D03 anchored at the `unique` constraint;
  test renamed to `validate_data_rich_reports_d01_d02_and_d03`.
- Clean fixture holds distinct values plus repeated NULLs and still exits 0,
  locking SQL UNIQUE NULL semantics end to end.
- README validate-data bullet lists D03; site/spec.md's `unique` bullet
  documents NULL exclusion and points at D03 in validation.md (the first
  per-check cross-reference on a constraint bullet).

Final state: 258 workspace tests green, clippy + fmt clean.

## key decisions

- **NULLs excluded from D03** (user decision): matches SQL UNIQUE semantics
  — NULLs compare as distinct, so an optional-but-unique column may
  legitimately hold many. Nulls in `required` columns remain D01's business;
  D02's repeated all-NULL-key counting is unchanged.
- **D02 overlap rule:** a column that is by itself the whole `primary_key`
  is not re-checked (D02 owns it); an explicit `unique` on a *composite*-key
  member IS checked individually — the explicit declaration wins, and D02's
  tuple check doesn't imply per-column uniqueness.
- **Third narrow trait method, not a generic query seam:** the phase-2
  blockquote from the previous session said to revisit at the third data
  check; revisited and kept — three methods each mapping 1:1 to a documented
  check (D01/D02/D03) is a cohesive seam, not a query catalogue, and fakes
  stay trivial. Reuse-with-a-bool-flag on `count_duplicate_keys` was
  rejected as less readable at both call sites.

## insights captured

- .claude-work/insights/20260706-1743-early-returns-starve-later-checks.md
  — function-level guard returns silently skip checks added later in the
  same function
- .claude-work/insights/20260707-0709-explicit-drop-flushes-duckdb-wal.md
  — explicit `drop(conn)` is load-bearing in e2e fixtures: it flushes
  DuckDB's WAL before the spawned binary's read-only open
