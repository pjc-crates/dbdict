# rich data level + DDL generator

## problem

The rich (0.2.0) format is complete at the spec and metadata levels, but two
gaps remain before it is useful end to end:

1. **`validate-data` on a rich dict does nothing.** It reports the honest
   `RichFormatUnsupported` pre-flight (`compare_dataset` in
   crates/dbdict/src/lib.rs) — the one user-visible gap left after the
   duckdb-spec session. The seam, source resolution, and table matching all
   exist from phase 3; the data level just needs queries against the real
   database.
2. **No generator exists yet**, even though generators are the reason for
   full type fidelity. The DDL generator is nearly free — the dict already
   round-trips through `CREATE TYPE`/`CREATE TABLE` in the scratch db, so the
   scratch DDL *is* the generator — and building it pressure-tests the public
   `load_and_lower` model API before more code accretes on top of it.

Two parked nits also land here because this session touches their areas:
the `dbdict spec | head` SIGPIPE panic (CLI) and the case-insensitive
table-name collision (S10 is exact while meta-level matching is
ASCII-folded).

## success criteria

- `dbdict validate-data` on a rich dict runs real data checks against the
  dict-level source database instead of the `RichFormatUnsupported`
  pre-flight:
  - **D01** (error): a `required` or `primary_key` column contains nulls.
  - **D02** (error, new code): a `primary_key` column (or composite key)
    contains duplicate values. Specced in site/validation.md before
    implementation.
- `dbdict ddl <dict>` prints executable DuckDB DDL (`CREATE TYPE` for
  typedefs + `CREATE TABLE` per table) to stdout, generated from the lowered
  model via `load_and_lower` — proven by executing the output against a fresh
  in-memory DuckDB and diffing `DESCRIBE` against the dict (the same
  round-trip trick validate-meta uses).
- `dbdict spec | head -1` exits cleanly instead of panicking (SIGPIPE reset
  in main).
- Case-insensitive table-name collisions are handled consistently: S10 and
  meta-level matching agree on one folding rule, with a test locking it in.
- `cargo test --workspace` green; clippy + rustfmt clean; site/validation.md
  documents D02 and any changed behaviour.

## scope

- in:
  - rich-path data level: D01 + D02 via the DuckDB backend seam
    (new data-level method(s) on the `DuckdbBackend` trait or a data seam
    in dbdict::rich)
  - new generator crate (e.g. crates/dbdict-ddl) consuming
    `dbdict::load_and_lower`; thin `dbdict ddl` CLI subcommand wiring
  - D02 spec text in site/validation.md
  - SIGPIPE fix in crates/dbdict-cli
  - S10 / meta-matching case-folding alignment
- out:
  - D02 on the legacy parquet path (legacy is preserved, not extended —
    it keeps its existing D01-only behaviour)
  - other generators (dummy data, Python/Julia codegen) — later sessions
  - fork branding ($learn_more URL, site/CNAME, site/index.md) — separate
    decision-heavy session; blocks only site publishing
  - any new rich-format schema surface

## constraints

- architecture rule: generators consume the model from `load_and_lower`
  only — the DDL crate never touches YAML, the CLI, or other crates'
  internals
- core `dbdict` crate stays duckdb-free; all DuckDB access goes through the
  backend trait seam
- legacy 0.1.0 path behaviour unchanged (205 existing tests keep passing)
- maintainer is learning Rust: training-wheels comments, no clever Rust
