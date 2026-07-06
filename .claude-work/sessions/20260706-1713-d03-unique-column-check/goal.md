# D03 unique column check

## problem

Columns declared `unique` that are not the primary key are never checked
against the data: D02 covers only the `primary_key` column set. This is now
the one declared constraint `validate-data` doesn't verify — a real gap,
because the DDL decision (2026-07-06, see the `generate` rustdoc in
crates/dbdict-ddl) commits dbdict to the declare-then-check model:
constraints are never emitted as SQL clauses, so a `unique` declaration is
worthless until something queries for violations.

## success criteria

- **D03** (error, rich format only): a column with the explicit `unique`
  constraint contains a non-NULL value that occurs in more than one row.
  - NULLs are excluded (`WHERE col IS NOT NULL` before grouping) — matches
    SQL UNIQUE semantics, where NULLs compare as distinct; an
    optional-but-unique column legitimately holds many NULLs. Nulls in
    `required` columns remain D01's business.
  - count reported = distinct duplicated values, mirroring D02.
- Specced in site/validation.md *before* implementation (the D02
  convention), including the NULL-exclusion rationale and the contrast
  with D02's NULL handling.
- `dbdict validate-data` reports D03 anchored at the column's `unique`
  constraint span; a seeded fixture fails, the clean fixture still passes.
- No double-reporting against D02: a column that is the table's *sole*
  `primary_key` column is D02's job; D03 checks explicitly-`unique`
  columns that aren't already covered by the D02 key. (An explicit
  `unique` on a member of a *composite* key IS checked individually —
  the explicit declaration wins, and D02's tuple check doesn't imply it.)
- `cargo test --workspace` green; clippy + rustfmt clean.

## scope

- in:
  - D03 spec text in site/validation.md
  - rich data level: D03 via the `DuckdbBackend` seam — either reuse
    `count_duplicate_keys` with a NULL filter or add one narrow method
    (design call for /ws plan; note the phase-2 blockquote said "revisit
    if a third data check makes the trait feel like a query catalogue" —
    this is the third check)
  - fake-backend tests in core, real-duckdb tests in dbdict-duckdb,
    CLI e2e snapshot update
  - README validate-data sentence mentions unique alongside D01/D02
- out:
  - legacy (0.1.0) path — preserved, not extended (same call as D02)
  - composite/multi-column `unique` keys — the format has no way to
    declare them (`unique` is per-column; composite uniqueness is
    `primary_key`'s job)
  - other generators, fork branding — separate sessions

## constraints

- core `dbdict` stays duckdb-free; queries live behind the backend trait
- rich queries use db-side name spellings; problems locate at dict spans
  (the established data-level pattern)
- legacy behaviour unchanged (existing tests keep passing)
- TDD, red first; maintainer is learning Rust — training-wheels comments
