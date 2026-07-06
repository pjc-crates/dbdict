# implementation: D04 referential integrity check

## phases

### phase 1: spec + S01 tightening + core check through the seam — DONE 2026-07-07T08:06:29+12:00
- [x] spec D04 in site/validation.md first (error, rich only): a
      `foreign_key` column contains a non-NULL value that does not exist
      in the `primary_key` column it is paired with by a relationship's
      *equality* conjunct. Document: NULL exclusion (SQL MATCH SIMPLE —
      NULL means "no reference"; nulls in `required` columns are D01's
      business); every declared fk→pk pair checked independently (one
      problem per violating pair, SQL analogue: each FK constraint stands
      alone); count = distinct orphaned values (mirrors D02/D03);
      self-joins work naturally. Update S01's wording in the same file to
      say the pairing conjunct must be an equality.
- [x] tighten S01 (crates/dbdict/src/validate_spec.rs, ~line 360): the
      resolution currently ignores the conjunct operator, so a `>=`
      conjunct satisfies it. Red test first: an fk column paired with a
      pk only by a range conjunct now gets S01.
      (fixture spec/s01-fk-range-conjunct-only.yaml + snapshot)
- [x] extract the fk→pk pairing resolution shared by S01 and D04 into
      one helper (proposed: a `DataDict` method in
      crates/dbdict/src/model.rs, e.g.
      `foreign_key_targets(table_name, column_name) -> Vec<FkTarget>`
      where `FkTarget` names the pk-side table and column — equality
      conjuncts only, pk side must carry `primary_key`). S01 becomes
      "targets is empty → error"; D04 iterates the targets. One
      resolution, impossible for the two checks to drift.
- [x] extend the `DuckdbBackend` trait (crates/dbdict/src/rich.rs) with
      the fourth narrow method:
      `count_orphaned_values(db_file, fk_table, fk_column, pk_table, pk_column) -> Result<usize, String>`

> decision (goal.md): narrow-method seam re-checked and kept at the
> fourth method — cross-table changes the SQL inside the method, not the
> seam's 1:1 check-to-query shape; fakes stay canned-count stubs.

- [x] implement in crates/dbdict-duckdb/src/native.rs (reuse
      `quote_ident`, `open_read_only`, `query_count`). Use
      `NOT EXISTS` (anti-join), NOT `NOT IN`: with `NOT IN`, a single
      NULL in the pk column makes the predicate NULL for every orphan
      and silently reports zero — `NOT EXISTS` is null-safe. Filter
      `WHERE fk.col IS NOT NULL` before the anti-join (the NULL-fk
      exclusion). Count `DISTINCT` fk values.
- [x] new `ProblemKind` variant + rendering, patterned on D03's
      (`OrphanedValues { count }` → code D04, Level::Data); message names
      the pk target so a multi-target column's problems are tellable
      apart
- [x] extend `check_table_data` in crates/dbdict/src/rich.rs: per
      db-present table, after D03 — for each `foreign_key` column, for
      each `FkTarget`, run D04; skip a target whose table is absent from
      the db (meta level already reports that); queries use db-side name
      spellings on *both* tables; the problem anchors at the column's
      `foreign_key` constraint span (new `foreign_key_span`, consistent
      with D01 `required` / D03 `unique` anchoring); query failures
      report M05-shaped `UnreadableSource` at the table
- [x] tests, red first:
      - S01 tightening (crates/dbdict/tests/ spec suite): range-only
        pairing now errors S01; equality pairing still satisfied
      - core fake-backend (crates/dbdict/tests/rich_data.rs): D04
        reported with count and pk target; clean fk column silent; NULL
        handling is the backend's job (fake asserts the call shape);
        two equality targets → two queries, two problems; a range
        conjunct alone → no D04 query (and S01 fires at spec level);
        self-join queries fk and pk on the same table; query failure →
        UnreadableSource; fk column in a db-absent target table → no
        query
      - real duckdb (crates/dbdict-duckdb/tests/data_queries.rs):
        orphans counted (distinct); NULL fk values excluded; a NULL in
        the *pk* column does not mask orphans (the NOT IN gotcha lock);
        case-insensitive identifier match; hostile-name quoting on both
        tables; self-join
- also: `check_table_data` now takes the whole dict and the full schema
      list — D04's pairing resolution needs the dictionary, and querying
      needs the pk-side tables' database spellings
- also: a *pk-side column* absent from the database is skipped too (its
      M02 already reported), alongside the planned absent-table skip;
      both have fake-backend tests
- also: `count_orphaned_values` re-exported at the dbdict-duckdb crate
      root next to the other query functions
- **verify:** `cargo test --workspace` green (274 passed, was 258: 1 new
  S01-tightening snapshot test, 8 new fake-backend tests incl. every-pair
  and range-conjunct-does-not-pair, 7 new real-duckdb tests incl. the
  NOT-IN-null-masking lock, self-join, and both-tables quoting); new
  tests demonstrably red before implementation; clippy + fmt clean
  — PASSED

### phase 2: CLI e2e + docs
- [ ] extend the rich-data CLI e2e fixtures (crates/dbdict-cli/tests/cli.rs):
      seeded fixture gains a second table + fk column with an orphaned
      value → snapshot shows D01+D02+D03+D04, D04 anchored at the
      `foreign_key` constraint (test renamed to match coverage; old
      snapshot deleted with the rename); clean fixture gains the same
      shape with all fk values present plus a NULL fk → still exits 0
      (locks NULL exclusion end to end)
- [ ] README.md: validate-data bullet mentions orphaned `foreign_key`
      values (D04) alongside D01–D03
- [ ] site/spec.md: the `foreign_key` constraint bullet and/or the
      relationships section points at D04 in validation.md (the D03
      cross-reference pattern)
- **verify:** `cargo test --workspace` green; snapshots reviewed before
  accepting; seeded fixture exits 1 reporting D01–D04, clean fixture
  exits 0; `cargo clippy --workspace --all-targets` + `cargo fmt --check`
  clean
