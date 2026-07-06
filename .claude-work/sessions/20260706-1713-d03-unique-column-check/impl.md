# implementation: D03 unique column check

## phases

### phase 1: spec + core check — D03 through the backend seam — DONE 2026-07-06T17:43:25+12:00
- [x] spec D03 in site/validation.md first (error, rich only): an
      explicitly-`unique` column contains a non-NULL value occurring in
      more than one row; count = distinct duplicated values (mirrors D02).
      Document the NULL exclusion (SQL UNIQUE semantics: NULLs compare as
      distinct; an optional-but-unique column legitimately holds many
      NULLs) and contrast with D02, which counts repeated all-NULL keys
      (nulls there are D01's business). Also state the D02 overlap rule:
      a column that is the table's *sole* `primary_key` column is D02's
      job and is not re-checked; an explicit `unique` on a *composite*-key
      member IS checked individually (D02's tuple check doesn't imply it)
- [x] extend the `DuckdbBackend` trait (crates/dbdict/src/rich.rs) with
      one narrow method:
      `count_duplicate_values(db_file, table, column) -> Result<usize, String>`
      — single column, `WHERE column IS NOT NULL` before grouping

> decision: a third narrow named method, not a pivot to a generic query
> seam. the phase-2 (previous session) blockquote said to revisit when a
> third data check lands; revisited — three methods each mapping 1:1 to a
> documented check (D01/D02/D03) is a cohesive seam, not a query
> catalogue, and fakes stay trivial. reuse-with-a-flag on
> `count_duplicate_keys` was rejected: a bool parameter would make both
> call sites less readable than two self-documenting methods.

- [x] implement in crates/dbdict-duckdb/src/native.rs (reuse
      `quote_ident`, `open_read_only`, `query_count`)
- [x] extend `check_data` in crates/dbdict/src/rich.rs: per db-present
      table, after D01/D02 — for each column with the explicit
      `Constraint::Unique`, skip when that column alone IS the primary-key
      set (D02 overlap rule), else run D03; queries use db-side name
      spellings; the problem anchors at the column's `unique` constraint
      span (consistent with D01 at `required`, D02 at the key column);
      query failures report M05-shaped `UnreadableSource` at the table
- [x] new `ProblemKind` variant + rendering, patterned on D02's
      (`DuplicateValues { count }` → code D03, Level::Data)
- [x] tests, red first:
      - core fake-backend (crates/dbdict/tests/rich_data.rs): D03
        reported with count; clean unique column silent; sole-pk column
        NOT queried for D03; composite-pk member with explicit `unique`
        IS queried; unique-without-required column never null-queried
        unless also required (D01 scope unchanged); query failure →
        UnreadableSource
      - real duckdb (crates/dbdict-duckdb/tests/data_queries.rs):
        duplicates counted; repeated NULLs NOT counted (the semantics
        lock); case-insensitive identifier match; hostile-name quoting
- also: D02's guard clauses restructured from function-level `return`s
      into scoped `if`s — the returns would have silently skipped D03 for
      pk-less tables (captured as an insight)
- also: the trait doc's "revisit at the third check" note now records the
      revisit: three 1:1 check-to-query methods is a cohesive seam,
      reconsider only if that mapping breaks
- **verify:** `cargo test --workspace` green (258 passed, was 248: 4 new
  real-duckdb query tests incl. the repeated-NULLs-not-duplicates lock,
  6 new fake-backend tests incl. sole-pk skip and composite-member check);
  new tests demonstrably red before implementation; clippy + fmt clean
  — PASSED

### phase 2: CLI e2e + docs
- [ ] extend the rich-data CLI e2e fixtures (crates/dbdict-cli/tests/cli.rs):
      seeded fixture gains a `unique` column with a duplicated value →
      snapshot shows D03 anchored at the `unique` constraint; clean
      fixture gains the same column with distinct values plus repeated
      NULLs → still passes (locks NULL exclusion end to end)
- [ ] README.md: validate-data bullet mentions duplicate `unique` values
      (D03) alongside D01/D02
- [ ] site/spec.md constraints section: `unique` line points at D03 the
      way `primary_key` behaviour is documented in validation.md
- **verify:** `cargo test --workspace` green; snapshots reviewed before
  accepting; seeded fixture exits 1 reporting D01+D02+D03, clean fixture
  exits 0; `cargo clippy --workspace --all-targets` + `cargo fmt --check`
  clean
