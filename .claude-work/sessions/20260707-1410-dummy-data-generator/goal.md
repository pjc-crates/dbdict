# dummy-data generator

## problem

dbdict can describe and validate a schema, but there is no way to produce
data *from* a dictionary. A dummy-data generator is the first consumer that
must *interpret* the type system (structs, enums, decimals, arrays) rather
than round-trip it, so it will shake out gaps in the core model API before
Python/Julia codegen builds on the same model.

Generated data must satisfy every declared constraint by construction:
D01 `required`, D02 `primary_key`, D03 `unique`, D04 `foreign_key`,
D05 `cardinality`. The existing validators become a built-in oracle —
`validate-data` against generated output should always pass. The
interesting design problem is satisfying constraints in a dependency
order (types → uniqueness → FK targets → cardinality bounds), especially
many-to-one range joins.

## success criteria

- `dbdict dummy` (name TBD in plan) generates a DuckDB database file from a
  `dbdict.yaml`, with `--rows N` (global, per-table override) and `--seed N`
  CLI flags; seeded runs are reproducible
- an export option emits the equivalent SQL INSERT statements for debugging
- generated databases pass `dbdict validate` and `dbdict validate-data`
  (all of D01–D05) by construction — verified in tests across the full
  rich-type surface: structs, enums, decimals, arrays, nested combinations
- crate split per the architecture rule (generators consume the core model,
  never YAML or each other):
  - `crates/dbdict-dummy-data` — backend-generic: orchestration, constraint
    satisfaction (uniqueness, FK selection, cardinality bounds), RNG/seeding
  - `crates/dbdict-dummy-data-duckdb` — DuckDB-specific: value generation
    mapped 1:1 to DuckDB column types, database writing
- wired as a CLI subcommand in `crates/dbdict-cli`
- workspace tests, clippy, fmt all green

## scope

- in: rich path (`dbdict.yaml`), full D01–D05 satisfaction, DuckDB file
  output, SQL INSERT export, `--rows`/`--seed` CLI config, deterministic
  seeded generation
- in (added during planning): a `duckdb: extensions:` dictionary section
  (parallel to `typedefs:`) declaring engine extensions; a check that
  each declared extension LOADs on the local engine (error if not);
  LOAD-only — network INSTALL is out of scope
- in (added during planning): type coverage is total over behavior —
  every canonical DuckDB type either generates (all built-ins +
  JSON/GEOMETRY/INET) or is refused with a descriptive error
- out: legacy path (`data-dict.yaml` / parquet) — stays validation-only;
  realistic/semantic data (names, addresses, distributions) — values need
  only be type- and constraint-correct; CSV/parquet output; generation
  config in the dictionary yaml itself

## constraints

- generators never touch YAML or the CLI internals — they consume the
  resolved model from `crates/dbdict` (`load_and_lower`), mirroring how
  `crates/dbdict-ddl` is structured
- the generic/duckdb split follows the data: anything keyed to concrete
  DuckDB column types lives in `dbdict-dummy-data-duckdb`; anything
  expressible against the resolved model alone lives in `dbdict-dummy-data`
- bundled DuckDB (v1.5.4, in-process) — no runtime duckdb on PATH
- maintainer is learning Rust: training-wheels comments, no fancy Rust,
  readability over speed
