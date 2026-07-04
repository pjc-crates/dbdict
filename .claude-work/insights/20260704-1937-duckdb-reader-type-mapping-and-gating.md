---
created: 2026-07-04T19:37:09+12:00
title: DuckDB shell-out reader — type mapping and feature gating
tags: [rust, duckdb, design, pattern, gotcha]
source: /ws done
---

## DuckDB DESCRIBE returns canonical, resolved type spellings — normalise + prefix-match

Verified against duckdb v1.5.4: `DESCRIBE` reports fully-resolved, parameterised
type strings, not the alias you wrote. `TIMESTAMPTZ` → `TIMESTAMP WITH TIME ZONE`,
`DECIMAL(9,2)` keeps its params, an enum column reports its whole inline
definition `ENUM('happy', 'sad')`, arrays render `INTEGER[]` / `INTEGER[3]`.

So the type map must normalise (uppercase, strip `(...)`/`[]`) and key off
**prefixes**, never exact equality: `starts_with("TIMESTAMP")` covers the whole
family (incl. `TIMESTAMP WITH TIME ZONE`, `TIMESTAMP_S/_MS/_NS`), `ENUM`/`DECIMAL`
prefixes, and a `contains('[')` check for arrays must run *before* the base match
or `INTEGER[]` wrongly maps to `number`. A naive exact-match table silently
misses every parameterised type. Mirrors how the parquet reader keys off resolved
logical/physical enums rather than raw strings.

## Shell-out reader contract

`duckdb -readonly -json <file> -c 'DESCRIBE "<table>";'` →
a JSON **array of row objects**, keys `column_name, column_type, null, key,
default, extra`. Read `column_name` + `column_type`.
- `-readonly`: never lock or mutate the user's database.
- `-c`: run one statement and exit (non-interactive).
- identifier goes in double-quotes, embedded `"` doubled.
- a missing table → duckdb exits non-zero → surfaced as a CLI error (map to M05).

## data-dict-duckdb mirrors data-dict-parquet's interface → link strategy is swappable

The new crate implements the same narrow interface as `data-dict-parquet`
(`describe` / `column_types`), so the core validation seam dispatches on source
kind without knowing the backend. This makes the bundled-vs-shell-out decision
*reversible*: we shipped shell-out (deps = `std` + `serde_json`, no arrow, no
C++), but a future bundled native reader can replace the internals behind the
same functions without touching the core. Pick the cheap option, keep the
interface honest, revisit only if reality demands.

## Feature gate is default-off, and gates the reader — not the schema

`duckdb` support is behind a `duckdb` cargo feature, off by default. The gate's
value is *not* build cost (shell-out is trivial to compile) — it's a capability
contract: an ungated build would make `data-dict` unconditionally willing to
spawn whatever `duckdb` binary is on PATH, which should be opt-in.

Gate the **reader**, not the schema: `schema.yaml`, the model, and lowering stay
ungated so `validate-spec` accepts and documents a duckdb source in any build;
only the `data-dict-duckdb` dep, the seam's duckdb branch, and `types duckdb` are
`#[cfg(feature = "duckdb")]`. Feature-off + duckdb source at meta → clear "not
supported in this build"; feature-on + duckdb missing → runtime "not on PATH".
Gotcha: `#[cfg]`'d code rots unless CI builds **both** configs.

## parquet-crate precedent: default-features=false drops arrow, but the workspace already compiles C

`data-dict-parquet` pulls `parquet` with `default-features = false` then re-adds
only the five compression codecs — deliberately excluding the heavy `arrow`
interop layer (confirmed: no arrow crates in the lockfile; the reader uses
parquet's own schema/record API). Corrected an earlier wrong assumption of mine:
the workspace is **not** pure-Rust — the `zstd` codec pulls `zstd-sys` + `cc`, so
a C compile already happens. (Bundled DuckDB would escalate that from a tiny C
lib to a large C++ engine — the real argument for keeping shell-out.)
