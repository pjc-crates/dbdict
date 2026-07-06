# implementation: rich data level + DDL generator

## phases

### phase 1: nits — SIGPIPE + S10 case folding — DONE 2026-07-06T13:57:19+12:00
small, independent warm-up; both areas get touched again later in the session.

- [x] reset SIGPIPE to default disposition at the top of `main` in
      crates/dbdict-cli/src/main.rs so `dbdict spec | head` dies quietly
      instead of panicking — `reset_sigpipe()` via
      `libc::signal(SIGPIPE, SIG_DFL)`, unix-only dependency with a no-op
      stub on windows. mechanism sourced from the rustc unstable book
      (on-broken-pipe page): rust sets SIGPIPE to SIG_IGN before main;
      SIG_DFL "is normally what you want if your program produces textual
      output". no maintained wrapper crate needed — one libc call
- [x] make S10 (crates/dbdict/src/validate_spec.rs) detect
      ASCII-case-insensitive collisions, matching `names_eq` in
      crates/dbdict/src/rich.rs — **rich documents only**: legacy names
      live in parquet, which is case-sensitive, so legacy S10 stays exact
      (locked in by a new regression test)

> decision: S10 folds to match the database's semantics, rather than making
> meta matching exact. two dict tables differing only in case cannot both
> exist in one DuckDB db, so the spec level should reject them up front.
> refined during implementation: the fold is gated on `Format::Rich` —
> folding legacy would wrongly reject valid parquet dicts.

- [x] tests: cli e2e for piped output (closes the read end pre-startup for
      determinism); rich snapshot tests for table + column case collisions;
      legacy exact-match regression lock
- also: column names fold too, not just table names — same duckdb
      identifier semantics; rich S10 messages explain the fold
- also: documented the format-dependent comparison in site/validation.md
- **verify:** `cargo test --workspace` green (209 passed); `target/debug/
  dbdict spec | head -1` exits 141 with empty stderr — PASSED

### phase 2: rich data level — D01 + D02 — DONE 2026-07-06T14:36:26+12:00
replaces the `RichFormatUnsupported` pre-flight with real data checks.

- [x] spec D02 in site/validation.md first (error): the primary-key
      column set of a table contains duplicate values — composite when
      multiple columns are marked `primary_key`, per SQL semantics; count
      = distinct duplicated key values. D01 text extended: legacy lists
      sample row numbers, rich reports a count (a live table has no
      stable row numbers). also replaced the "data level not built yet"
      note with how the rich data level works
- [x] extend the `DuckdbBackend` trait (crates/dbdict/src/rich.rs) with
      data-level methods, keeping core duckdb-free:
      - `count_nulls(db_file, table, column) -> Result<usize, String>`
      - `count_duplicate_keys(db_file, table, key_columns) -> Result<usize, String>`

> decision: explicit named methods over a generic `query_count(sql)` seam —
> core must not build SQL strings (identifier quoting is backend knowledge),
> and two narrow methods are easier to fake in tests. revisit if a third
> data check makes the trait feel like a query catalogue.

- [x] implement both methods in crates/dbdict-duckdb/src/native.rs
      (reuse `quote_ident`; extracted `open_read_only` shared with
      `read_schema`; `query_count` helper)
- [x] new `check_data` in crates/dbdict/src/rich.rs mirroring `check_meta`:
      runs `check_meta` first, then re-resolves the source *quietly*
      (M04/M05/M06 already reported) → for each db-present table run D01
      per required/primary_key column and D02 once over the primary-key
      column set; queries use the database's own name spellings, problems
      locate at dictionary spans; query failures report as M05-shaped
      `UnreadableSource` at the table
- [x] `validate_data` now takes `&dyn DuckdbBackend` (mirrors
      `validate_meta`); rich routes to `rich::check_data`; deleted
      `compare_dataset` and the `RichFormatUnsupported` variant + its
      transitional test; CLI passes `NativeDuckdb`
- [x] tests: 8 fake-backend tests in tests/rich_data.rs (D01/D02/composite/
      clean/only-required-queried/no-pk-no-query/query-failure/missing-table);
      7 real-duckdb tests in dbdict-duckdb tests/data_queries.rs (incl.
      case-insensitive identifier match and hostile-name quoting); 2 cli
      e2e tests with reviewed snapshot (D01 anchored at the `required`
      constraint, D02 at the key column)
- also: legacy validate_data tests now pass `&NoDuckdb`; rich_meta fakes
      gained unreachable data-method stubs (meta must never query values);
      README validate-data paragraph updated; stale `compare_dataset` doc
      references fixed
- **verify:** `cargo test --workspace` green (225 passed); `dbdict
  validate-data` on the seeded fixture reports D01 + D02, exits 1; clean
  fixture exits 0; legacy parquet fixtures unchanged — PASSED

### phase 3: DDL generator — crates/dbdict-ddl + `dbdict ddl`
first generator; pressure-tests the public `load_and_lower` model API.

- [ ] new crate `crates/dbdict-ddl` (workspace member): library exposing
      `generate(dict: &DataDict) -> Result<String, ...>` producing
      executable DuckDB DDL — `CREATE TYPE` per typedef then `CREATE TABLE`
      per table — consuming the lowered model only (never YAML, never the CLI)

> decision: dbdict-ddl depends on dbdict (model) AND dbdict-duckdb
> (backend). typedefs may reference each other, and a flat script needs
> them in dependency order; rather than growing a topological sorter over
> type expressions (a grammar we deliberately don't own), reuse the
> fixpoint trick: execute candidate CREATE TYPE statements against a
> scratch db and emit them in the order that succeeded. same
> "compare outcomes, don't track dependency edges" principle as phase 4's
> resolve fix.

> decision: table-scoped typedefs that shadow a global (or collide across
> tables) cannot exist in one flat script's namespace. v1 policy: emit a
> clear error listing the shadowing typedefs and refuse to generate,
> rather than inventing a renaming scheme nobody asked for yet.

- [ ] round-trip self-check inside generation or tests: execute the
      generated script against a fresh in-memory DuckDB and diff canonical
      `DESCRIBE` output against the dict's own instantiation (the
      validate-meta trick) — proves the script is executable and faithful
- [ ] wire `dbdict ddl <dict>` in crates/dbdict-cli/src/main.rs: thin flat
      subcommand printing the script to stdout; problems (load errors,
      shadowing refusal) go to stderr with nonzero exit like other commands
- [ ] docs: short `ddl` section in README.md and site (wherever resolve /
      types duckdb are documented)
- [ ] tests: dbdict-ddl round-trip integration tests (typedef chains,
      structs/enums/decimals/arrays, shadowing refusal); cli e2e snapshot
- **verify:** `cargo test --workspace` green; `dbdict ddl` output piped
  into `duckdb` scratch recreates a schema whose DESCRIBE matches
  `dbdict resolve`; `cargo clippy --workspace` + `cargo fmt --check` clean
