# D04 referential integrity check

## problem

`foreign_key` is the last declaration `validate-data` never verifies against
the data. S01–S03 check that fk declarations are internally consistent (a
relationship exists and references real tables/columns), but nothing queries
for *orphaned values* — fk values with no matching row in the referenced
`primary_key` column. Under the declare-then-check model this matters the
same way D03 did: constraints are never installed in the database (types-only
DDL), so a `foreign_key` declaration is worthless until something runs SQL
that checks it. To be explicit: D04 adds **no constraints to the DuckDB
database** — it runs read-only queries and reports violations as
diagnostics, like D01–D03.

## success criteria

- **D04** (error, rich format only): a `foreign_key` column contains a
  non-NULL value that does not exist in the `primary_key` column it is
  paired with by a relationship's equality conjunct.
  - Pairing follows S01's resolution: the fk column on one side of an `=`
    conjunct, a `primary_key`-constrained column on the other. **Every**
    declared fk→pk pair is checked independently — one D04 problem per
    violating pair (SQL analogue: each FK constraint stands alone).
  - NULL fk values are excluded (`WHERE col IS NOT NULL`) — SQL
    MATCH SIMPLE semantics: NULL means "no reference". Nulls in `required`
    columns remain D01's business. Mirrors the D03 NULL decision.
  - count reported = distinct orphaned values (mirrors D02/D03).
  - Self-joins (fk referencing a pk in the same table) work naturally.
- Specced in site/validation.md *before* implementation (the D02/D03
  convention), including the NULL rationale and the every-pair rule.
- `dbdict validate-data` reports D04 anchored at the column's
  `foreign_key` constraint span (consistent with D01/D03 anchoring);
  a seeded fixture fails, the clean fixture still passes.
- `cargo test --workspace` green; clippy + rustfmt clean.

> decision (user, 2026-07-07): S01's resolution (validate_spec.rs:360)
> currently ignores the conjunct *operator* — a `>=` conjunct pairing fk
> with pk satisfies S01 today. A range predicate is not a reference, so
> this session tightens S01 to equality conjuncts and D04 pairs the same
> way — both checks resolve fk→pk identically.

> decision (user, 2026-07-07): the narrow-method seam survives the
> fourth method. `count_orphaned_values` is cross-table, but that changes
> the SQL *inside* the method (anti-join), not the seam's shape — still
> one self-describing method per documented check, fakes stay trivial
> canned-count stubs, and no SQL leaks into core. This re-checks and
> keeps the D03-session decision ("reconsider if the 1:1 mapping
> breaks").

## scope

- in:
  - D04 spec text in site/validation.md
  - rich data level: a fourth narrow `DuckdbBackend` method (anti-join
    count: `count_orphaned_values(db_file, fk_table, fk_column,
    pk_table, pk_column)`) — seam decision re-checked and kept, see
    blockquote above
  - pairing resolution in core `check_data` (dict side), reusing/matching
    S01's resolution logic
  - S01 tightened to equality conjuncts (see decision above)
  - fake-backend tests in core, real-duckdb tests in dbdict-duckdb
    (incl. NULL exclusion lock, self-join, hostile-name quoting),
    CLI e2e snapshot update
  - README validate-data sentence mentions D04 alongside D01–D03
- out:
  - composite-fk *tuple* semantics — each fk column pair is checked
    independently, per S01's per-column model; the format has no way to
    declare a tuple fk
  - range conjuncts (`>=`, `<=`, `>`, `<`) — never define an fk pairing
  - cardinality verification (one-to-one / many-to-one against data) —
    a plausible future check, separate session
  - legacy (0.1.0) path — preserved, not extended (same call as D02/D03)

## constraints

- **no constraints installed in the database** — read-only SQL checks
  only; the types-only DDL decision stands
- core `dbdict` stays duckdb-free; queries live behind the backend trait
- rich queries use db-side name spellings; problems locate at dict spans
- legacy behaviour unchanged (existing tests keep passing)
- TDD, red first; maintainer is learning Rust — training-wheels comments
