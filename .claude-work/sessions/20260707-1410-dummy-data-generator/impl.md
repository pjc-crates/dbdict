# implementation: dummy-data generator

## design overview

Two new crates, mirroring how `dbdict-ddl` consumes the model:

- `crates/dbdict-dummy-data` — backend-generic. Turns a `DataDict` +
  options into a **generation plan**: table order (topological by FK
  dependency), per-table row counts, and a per-column *role* describing
  what its values must satisfy (unique key, FK draw, range-join slot,
  plain fill with optional NULLs). Deps: `dbdict` only (+ `rand`).
  Knows nothing about concrete DuckDB types.
- `crates/dbdict-dummy-data-duckdb` — DuckDB-specific. Maps canonical
  DuckDB type strings (from `dbdict_duckdb::instantiate`, since the model
  carries `col_type` only as a raw string — there is no type AST) to
  value generators, renders rows as SQL INSERT literals, executes the
  script on a writable connection into a `.duckdb` file, and can emit the
  same script as a `.sql` debug export. Deps: `dbdict`,
  `dbdict-dummy-data`, `dbdict-ddl` (schema script), `dbdict-duckdb`
  (`quote_ident`, scratch helpers), `duckdb`.

> value strategy: SQL INSERT literals executed via `Connection`, not the
> Appender API. Simpler, directly satisfies the SQL-export requirement
> (the export *is* the script we execute), and readable when debugging.
> Speed is explicitly not a goal.

**Extension declarations (new core feature, phase 1).** The dictionary
gains a top-level section parallel to `typedefs:`:

```yaml
duckdb:
  extensions:
    - json
    - spatial
```

Declared extensions are LOADed into every engine connection dbdict opens
for that dictionary (scratch/instantiate, validate, generator). A new
engine-level check verifies each declared extension actually loads on
the local engine; failure is a named validation error. Per DuckDB
semantics: INSTALL is per-machine/per-version (persists a binary under
the extension directory), LOAD is per-process — so "available" means
"this process can LOAD it".

> LOAD-only policy this session: we never INSTALL (network fetch to
> extensions.duckdb.org) — extensions must already be installed or
> statically linked. `open_scratch()` already sets
> `enable_external_access(false)` for hostile-input hardening and
> sandboxed runs must not reach the network. an explicit
> `--install-extensions` opt-in is future work, noted not built.

**Core value trick — deterministic indexed generation.** Every value is a
pure function `nth(seed, table, column, i) → literal`, injective in `i`
and *monotone* in `i` for orderable types. That gives:
- uniqueness by construction (D02/D03): distinct `i` → distinct value,
  no rejection sampling
- FK correctness (D04) without reading data back: an FK draw is "pick a
  target row index `k`, emit the target PK's `nth(k)`" — both sides
  compute the same literal
- range joins (D05): monotone `nth` turns interval construction into
  index arithmetic — "one"-side row `k` owns slot `[2k, 2k+1]`, probe
  values land strictly inside one slot

Finite-capacity types (BOOLEAN, ENUM) report a capacity; asking for more
unique values than capacity is a clear error, not a wraparound.

**Type coverage — total over behavior, not over types.** The dictionary
can legally contain any type string DuckDB's `DESCRIBE` accepts
(including extension types), so "all possible types" is not a closable
list. The guarantee instead: every canonical type string either
*generates* or is *refused with a descriptive error* — never a panic or
a silently-wrong value.
- generate: all built-in general-purpose types and all nested types per
  the DuckDB docs (https://duckdb.org/docs/current/sql/data_types/overview.html)
  — int family incl. unsigned + HUGEINT/UHUGEINT, FLOAT/DOUBLE,
  DECIMAL(p,s), VARCHAR, BLOB, BIT, BOOLEAN, DATE, TIME, TIMESTAMP,
  TIMESTAMPTZ, INTERVAL, UUID, ENUM, plus LIST (`T[]`), fixed-length
  ARRAY (`T[N]`), STRUCT, MAP, UNION — recursively composable
- generate: effectively-first-class extension types — `JSON` (bundled
  json extension; physically VARCHAR with JSON semantics), `GEOMETRY`
  incl. the CRS-parameterized form like `GEOMETRY('EPSG:2193')`
  (built-in data type since duckdb v1.5), and `INET` (core inet
  extension). loadability is guaranteed by the phase 1 extensions
  check (dict declares them, dbdict LOADs them); literal syntax and
  bundled-engine behavior settled empirically by phase 2 probe tests —
  anything the bundled 1.5.4 engine rejects stays `Unsupported`,
  documented either way
- nothing needed for vss vector columns: no new logical type — vss
  indexes plain `FLOAT[N]` ARRAY columns, covered by construction
- refuse: any other extension type and anything unrecognized, via an
  explicit `Unsupported(raw)` parser fallback
- non-orderable types (MAP, UNION, BIT, JSON, GEOMETRY, nested
  containers) are injective-only — usable for unique/FK, refused as
  range-join bounds

> bundled engine is duckdb 1.5.4; the docs page tracks current. phase 2
> round-trip tests against the bundled engine are ground truth for what
> generates — newer doc-listed types (BIGNUM, VARIANT) land in
> `Unsupported` if 1.5.4 rejects them

**Constraint-satisfaction order** (per the D01–D05 contract in
`crates/dbdict/src/rich.rs`): types → D01 no NULLs in
required/PK columns → D02 unique PK tuples → D03 unique non-NULL values
→ D04 FK ∈ target PK values (or NULL) → D05 at most one match on each
declared "one" side. The oracle is `validate_data` itself — every
generated database must pass it in tests.

> unsupported shapes (e.g. join conjuncts the slot scheme can't satisfy,
> FK cycles, unique enum column with rows > variants) are refused with a
> descriptive error, mirroring `DdlError`'s style — never silently wrong

## phases

### phase 1: duckdb extensions declaration (core) — DONE 2026-07-07T17:12:27+12:00

- [x] model (`crates/dbdict/src/model.rs`): `extensions:
      Vec<Spanned<String>>` on `DataDict` (empty when the section is
      absent); lower from a top-level `duckdb: extensions: [...]` YAML
      section (`crates/dbdict/src/lower.rs`); schema-0.2.yaml gained the
      closed `duckdb:` object (`anyOf: [string, null]` items so an empty
      entry reaches S19 with a clear message)
- [x] spec validation (`validate_spec`): S19 error — name must be
      non-empty lowercase `[a-z0-9_]` (doubles as LOAD-interpolation
      safety: dictionaries are untrusted input); S20 warning — duplicate
      declaration; rich format only (legacy schema rejects the key)
- [x] engine check: M10 ("declared extension does not load on this
      engine") via new `DuckdbBackend::load_extensions` trait method with
      a default body, so the seven test fakes needed no changes; settled
      empirically as a plain LOAD attempt on a scratch connection —
      reported before instantiation (root cause of downstream M09s)
- [x] wire LOAD of declared extensions into the connections dbdict opens
      for a dictionary: `instantiate`/`open_scratch` paths in
      `crates/dbdict-duckdb/src/native.rs` — LOAD-only, never INSTALL;
      backend re-checks the name charset defensively
- [x] tests: lowering + spec-validation unit tests; engine-check tests
      (declared-and-loadable passes, bogus extension name errors);
      existing suites untouched when no section is declared
- [x] docs: site/spec.md "DuckDB extensions" section; site/validation.md
      S19/S20/M10 entries
- also: added the `json` cargo feature to the workspace duckdb dep —
  empirical finding: `enable_external_access(false)` blocks loading
  external extension *binaries*, so only statically-linked extensions
  can load in validation; the crate offers json/parquet/icu features but
  no spatial/inet, so those report M10 honestly on this build
- also: real-engine probes for json landed early (part of phase 2's
  probe list): `crates/dbdict-duckdb/tests/extensions.rs` proves
  `LOAD json` succeeds under the hardened config and a `JSON` column
  canonicalizes when declared; GEOMETRY/INET probes remain for phase 2
- **verify:** `cargo test --workspace`, clippy, fmt; a fixture dict
  declaring `json` validates cleanly, one declaring a bogus extension
  fails with the named error

### phase 2: generator scaffolding + typed value generation

- [ ] add `crates/dbdict-dummy-data` and `crates/dbdict-dummy-data-duckdb`
      to root `Cargo.toml` members + `[workspace.dependencies]`; create
      both crates (`dbdict-dummy-data` as a stub with error type)
- [ ] `dbdict-dummy-data-duckdb/src/types.rs`: small paren-aware parser
      for canonical type strings → `DuckType` — full built-in surface per
      the coverage list in the design overview (scalars incl. unsigned/
      HUGEINT/BIT/BLOB/INTERVAL/UUID, `DECIMAL(p,s)`, `ENUM(...)`,
      `STRUCT(...)`, `MAP(...)`, `UNION(...)`, `T[]` lists, `T[N]`
      arrays — recursive) + explicit `Unsupported(raw)` fallback
- [ ] `dbdict-dummy-data-duckdb/src/values.rs`: `nth(i)` literal
      generation per `DuckType` — injective always, monotone for
      orderable scalars (ints, floats, decimals, VARCHAR, DATE, TIME,
      TIMESTAMP/TZ, INTERVAL), capacity reporting for BOOLEAN/ENUM;
      recursive for nested containers; `Unsupported` → descriptive
      error; seed folded in for plain (non-key) variation
- [ ] extension-type probe: one test each for `JSON`,
      `GEOMETRY`/`GEOMETRY('EPSG:2193')`, `INET` — declared via the
      phase 1 section, do they instantiate on the bundled engine? wire
      the outcome: generates (injective literals; JSON = valid-JSON
      strings, literal syntax for GEOMETRY/INET settled empirically
      here) or stays `Unsupported`; record the result in a comment +
      this file
- [ ] unit tests: for each supported type, CREATE a scratch table with
      that column type, INSERT a run of generated literals, assert row
      count + no cast errors; assert monotonicity/injectivity where
      claimed; include an explicitly nested fixture
      (struct-in-struct with list members) to prove the recursion at the
      value level
- **verify:** `cargo test -p dbdict-dummy-data-duckdb`,
  `cargo clippy --workspace`, `cargo fmt --check`; workspace still builds

### phase 3: generation plan (dbdict-dummy-data)

- [ ] `GenerateOptions`: global row count, per-table overrides
      (`HashMap<String, u64>`), seed, fixed NULL fraction for optional
      columns (deterministic which rows get NULL)
- [ ] plan builder `plan(dict, &opts) -> Result<Plan, DummyDataError>`:
      refuse `Format::Legacy`; topo-order tables via
      `DataDict::foreign_key_targets` (FK cycle → error); per-column
      roles: composite-PK/unique → indexed-unique, FK → draw from target
      (injective draw when the FK column itself must be unique, e.g.
      one-to-one or `unique` FK), else plain fill (NULLs only if not
      `is_required_implied`)
- [ ] cardinality analysis (equality joins): many-to-one/one-to-many
      satisfied when the "one" side join columns are unique — verify or
      refuse; one-to-one → both sides injective; mark range-join
      (`Ge/Le/Gt/Lt`) relationships in the plan as slot-based (semantics
      implemented in phase 5, refused until then)
- [ ] unit tests with hand-built fixtures (`SourceInfo::for_test`,
      mirroring `crates/dbdict-ddl/tests/generate.rs` helpers): table
      order, role assignment, legacy/cycle/capacity errors
- **verify:** `cargo test -p dbdict-dummy-data`; clippy + fmt clean

### phase 4: end-to-end generation + D01–D05 oracle (equality joins)

- [ ] `dbdict-dummy-data-duckdb/src/lib.rs`:
      `generate(dict, &opts) -> Result<Generated, ...>` — schema via
      `dbdict_ddl::generate`, rows from plan + `nth` values, full script
      = DDL + INSERTs; `Generated` carries the script and a
      `write_db(path)` step executing it on a writable
      `Connection::open(path)` (declared extensions LOADed first)
- [ ] integration tests: fixture dicts covering the rich-type surface
      (structs, enums, decimals, arrays, nested) with all of
      required/primary_key/unique/foreign_key + equality-join
      relationships; generate → run `dbdict::validate_data` with
      `NativeDuckdb` as the oracle → status passes, zero problems
- [ ] determinism test: same seed → byte-identical script; different
      seed → different plain values
- **verify:** `cargo test --workspace` green; oracle tests prove
  D01–D04 + equality D05 by construction

### phase 5: D05 range joins + one-to-one

- [ ] slot-based generation for range conjuncts: "one"-side row `k` gets
      a closed interval from monotone `nth` (e.g. bounds at indices
      `2k`/`2k+1`), non-overlapping by construction; "many"-side probe
      values placed strictly inside a chosen slot; handle `Gt/Lt`
      open bounds and multi-conjunct joins (equality conjuncts pin the
      slot owner, range conjuncts use its interval)
- [ ] refuse (descriptive error) join shapes outside the scheme —
      e.g. range join where the bound columns are also FK/unique in
      conflicting ways
- [ ] oracle tests: many-to-one range join (the motivating case),
      one-to-one checked in both directions, multi-conjunct mix;
      all pass `validate_data`
- **verify:** `cargo test --workspace`; D05 oracle fixtures green
  including range joins

### phase 6: CLI subcommand + SQL export + docs

- [ ] `crates/dbdict-cli/src/main.rs`: `Command::Dummy { dict, rows,
      table_rows, seed, out, sql, force }` + `run_dummy` mirroring
      `run_ddl` (`main.rs:302`): load_and_lower → render warnings →
      generate → write `--out <file.duckdb>` (refuse existing unless
      `--force`), optional `--sql <file.sql>` export
- [ ] flags: `--rows N` (global default, default 10), `--rows-table
      TABLE=N` (repeatable), `--seed N` (default 0)
- [ ] e2e tests in `crates/dbdict-cli/tests/cli.rs`
      (`CARGO_BIN_EXE_dbdict` + insta pattern): happy path (generate then
      `validate-data` the output via the CLI), sql export, refuse-existing,
      legacy refusal; update `no_args_lists_all_subcommands` snapshot
- [ ] docs: README command listing; site page for the generator if the
      site structure has a natural home (check `site/` during the phase)
- **verify:** `cargo test --workspace`, clippy, fmt; manual smoke:
  `dbdict dummy examples/... -o /tmp/x.duckdb && dbdict validate-data ...`

> --out is always explicit; we never default to the dictionary's declared
> `source.file` — too easy to clobber a real database

> --install-extensions (network INSTALL opt-in) deliberately not built
> this session — LOAD-only everywhere; queue for a later session
