# dbdict

`dbdict` is a data dictionary tool for DuckDB databases. A `dbdict.yaml` file
describes a database's tables and columns — types, constraints, relationships,
and the domain vocabulary you need to understand them — in a single file that
humans and AI agents can co-author, and the CLI validates that the dictionary
and the real database still agree.

> This repository is a fork of
> [tidyverse/data-dict](https://github.com/tidyverse/data-dict) (MIT),
> deliberately diverged: it is not tracking upstream and not aiming for
> cross-backend portability. DuckDB-first.

Columns are typed in DuckDB's own type system — `STRUCT`, `ENUM`, `LIST`,
arrays, `DECIMAL(p,s)`, and so on — with a `typedef:` alias layer for naming
and reusing types:

```yaml
$version: "0.2.0"
typedef:
  money: DECIMAL(18, 4)
  address: STRUCT(city VARCHAR, postcode INTEGER)
source:
  duckdb:
    file: warehouse.duckdb
tables:
  - name: trades
    columns:
      - name: qty
        type: BIGINT
      - name: price
        type: money
```

Validation round-trips the dictionary through a scratch in-memory DuckDB and
compares `DESCRIBE` output against the real database, so type checking is
exact — struct fields, enum values, decimal precision, array sizes — with
source-span diagnostics pointing back into the YAML.

Two formats are supported, selected by `$version` (see
[`site/spec.md`](site/spec.md)):

* **`0.2.0` (rich)** — DuckDB-native types + `typedef:`, one dictionary per
  DuckDB database. The current direction.
* **`0.1.0` (legacy)** — the upstream `data-dict.yaml` format (coarse semantic
  types validated against per-table Parquet files), preserved so existing
  files keep validating.

## The CLI

The `dbdict` CLI validates dictionaries at three levels (see
[`site/validation.md`](site/validation.md)) and ships a few helpers:

```
Usage: dbdict <COMMAND>

Commands:
  validate-spec  Validate a dbdict.yaml file or directory against the spec [default: .]
  validate-meta  Validate a dataset's column names and types against a data dictionary
  validate-data  Validate a dataset's values against a data dictionary
  resolve        Print each typedef's canonical DuckDB expansion [default: .]
  spec           Print the dbdict.yaml specification
  types parquet  Print column types for a parquet file
  types duckdb   Print every table's column types from a DuckDB database
  skill read     Skill for reading and understanding a data dictionary
  skill write    Skill for creating or updating a data dictionary
  help           Print this message or the help of the given subcommand(s)
```

* `validate-spec` checks that a file is structurally valid and internally
  consistent. Pass a file, or a directory containing a `dbdict.yaml` (falls
  back to the legacy `data-dict.yaml` name; defaults to the current
  directory).
* `validate-meta` compares a dictionary against its database's column names
  and types; `validate-data` also checks values — nulls in `required` columns
  (D01) and duplicated `primary_key` values (D02). The data is located through
  the dictionary's `source`, so only the dictionary is passed.
* `resolve` expands every `typedef:` alias to its canonical DuckDB spelling —
  useful while authoring, and for seeing exactly what validation compares. (A
  legacy dictionary has no typedefs, so it resolves to nothing.)
* `types duckdb` / `types parquet` print the column types of a data source.
* `skill read` / `skill write` print embedded agent skills for working with
  data dictionaries, and `spec` prints the full specification.

DuckDB is bundled into the binary (native, in-process): nothing needs to be
installed on `PATH`, and the dictionary's database is always opened read-only.

### Install

Build and install from source with [Cargo](https://rustup.rs):

```bash
cargo install --git https://github.com/pjc-wspace/dbdict dbdict-cli
```

Or clone the repo and build locally:

```bash
git clone https://github.com/pjc-wspace/dbdict.git
cd dbdict
cargo build --workspace --release
# binary is at target/release/dbdict
```

The first build compiles the bundled DuckDB (C++), which takes several
minutes; subsequent builds are cached.

## Development

This is a Rust workspace of library crates plus a thin CLI. The core is a pure
library exposing the parsed, resolved dictionary model; the CLI and future
generators (dummy data, SQL/DDL, client codegen) are separate crates that
consume that model:

* `crates/dbdict/` — core: model, source-mapped YAML parsing, validation
  engine, diagnostics. Free of any DuckDB dependency.
* `crates/dbdict-duckdb/` — DuckDB backend (native bundled `duckdb` crate):
  scratch instantiation, schema reading, typedef expansion.
* `crates/dbdict-parquet/` — Parquet backend for the legacy format.
* `crates/dbdict-cli/` — thin CLI wrapper (binary: `dbdict`).

```bash
cargo build --workspace
cargo test --workspace
```

The website is a [Quarto](https://quarto.org) project in [`site/`](site/),
published to GitHub Pages on every push to `main`.
