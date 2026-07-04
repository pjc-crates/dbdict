# dbdict

A data dictionary tool for databases. Forked from tidyverse `data-dict` and
deliberately diverged — not tracking upstream, not aiming for cross-backend
portability.

## What it does
- **Rich path (the new direction):** `dbdict.yaml` describes tables and columns
  in DuckDB-native types via a `typedef:` alias layer. Validation round-trips the
  dict through an in-memory DuckDB and compares `DESCRIBE` output against the real
  database, so type fidelity is exact (structs, enums, decimals, arrays).
- **Legacy path (preserved):** `data-dict.yaml` (coarse semantic types + parquet)
  still validates — kept so existing/upstream files keep working.

## Architecture
Workspace of library crates + a thin CLI. The core (`dbdict`) is a pure library
exposing the parsed/resolved dictionary model; the CLI and future generators
(dummy data, SQL/DDL, Python/Julia codegen) are separate crates that consume that
model — they never touch YAML or each other.
- `crates/dbdict` — core: model, source-mapped YAML parse, typedef resolution,
  validation engine, diagnostics
- `crates/dbdict-duckdb` — DuckDB backend (native bundled `duckdb` crate)
- `crates/dbdict-parquet` — parquet backend (legacy path)
- `crates/dbdict-cli` — thin CLI (binary: `dbdict`)

## Coding conventions
- **The maintainer is learning Rust.** Write "training-wheels" comments: thorough
  but concise, explaining the *why* and any Rust idiom or gotcha in play
- lowercase comments; no trailing period on end-of-line comments
- **no fancy/clever Rust** — keep it explicit and readable (avoid dense iterator
  chains, macro tricks, and lifetime gymnastics when a plain version is clearer)
- optimize for readability and maintenance / feature-addition cost over execution
  speed
- follow `rustfmt` defaults

## Build / test
- `cargo build --workspace` / `cargo test --workspace`
- DuckDB is bundled (native, v1.5.4) — no runtime `duckdb` on PATH needed by the
  library. Some transitional shell-out tests still call the `duckdb` CLI; those go
  away when the duckdb reader is reworked onto the native crate.
