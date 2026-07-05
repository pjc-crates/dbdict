---
created: 2026-07-05T10:04:55+12:00
title: DuckDB does the typedef substitution — round-trip validation
tags: [rust, duckdb, design, pattern]
source: /state save
---

## DuckDB CREATE TYPE + DESCRIBE does all the alias work — we do zero substitution

Verified (duckdb 1.5.4, in-process native crate): DuckDB supports type aliases
(`CREATE TYPE x AS INTEGER | STRUCT(...) | FLOAT[768] | ...`), they **compound**
(an alias referenced inside another resolves), and `DESCRIBE`/`typeof`
canonicalise the result. So to validate a dict's declared types against a real
db, **round-trip**: `CREATE TYPE` the typedefs + `CREATE TABLE` the dict's tables
into a scratch in-memory connection, `DESCRIBE` them → canonical `(name, type)`,
and compare to the real db's `DESCRIBE`. Both sides are DESCRIBE output from the
same engine, so they byte-match for identical logical types (proven across
struct/array/map/union/decimal/enum). No hand-rolled type-grammar parser, no
substitution engine, no canonicalisation to reimplement — DuckDB owns it. That's
why the alias feature is small.

## Post-pivot, native bundled duckdb beats shell-out — every shell-out reason evaporated

Shell-out was chosen to keep the default build pure / avoid forcing duckdb on
non-duckdb users / stay upstream-friendly. The pivot to duckdb-primary + dropping
upstream deleted all three. Native bundled `duckdb` crate (official, `1.10504.x`
= DuckDB 1.5.4) gives in-process `Connection::open_in_memory()` (ideal for the
round-trip), a self-contained binary (no `duckdb` on PATH), and drops the feature
gate. First bundled build ~60s; it pulls arrow + reqwest (trim later with
`default-features = false`).

## Divergence of goals ≠ divergence of code (fork + license)

Our merge-base with upstream is the tidyverse HEAD — the entire engine (S-checks,
YAML/diagnostics, the parquet path we KEEP) is their MIT code plus our commits on
top. So the fork relationship is honest provenance, not stale — keep it. Upstream
declares MIT only via `Cargo.toml` metadata (SPDX name, no LICENSE file/holder):
a valid grant by reference, but a LICENSE *file* must reproduce the full text +
copyright notice (MIT's own "shall be included" clause requires it), which we
added. See [[rust-training-wheels-comments]] for the code-style side.
