---
created: 2026-07-05T10:04:55+12:00
title: dbdict pivot — native duckdb, rename, phase 1 done
tags: [rust, duckdb, design, workflow]
summary: Fork data-dict → dbdict; pivoted to rich DuckDB-native types via typedef aliases validated by round-trip through in-memory duckdb (mechanism C proven). Renamed workspace + repo, added native bundled duckdb crate, LICENSE. Next is phase 2 (typedef schema/parsing); 3 commits unpushed.
---

## Goal
Build `dbdict` — a rich, DuckDB-native data dictionary tool (fork of
tidyverse/data-dict). Columns are typed in DuckDB-native types via a `typedef:`
alias layer; validation round-trips the dict through an in-memory DuckDB and
compares `DESCRIBE` output against the real database (full type fidelity).
**Dual-format:** legacy `data-dict.yaml` (coarse types + parquet) preserved; new
`dbdict.yaml` (rich types + duckdb) added alongside.

## Current State
- **Phase 1 DONE** — `dbdict-duckdb` reader crate. The round-trip mechanism (C)
  is PROVEN: `crates/dbdict-duckdb/tests/spike_roundtrip.rs` shows a table typed
  with `typedef` aliases produces a `DESCRIBE` that byte-matches the same table
  typed with native types, across struct (+compounding), fixed/var arrays,
  decimal, enum, map. Native bundled `duckdb` crate compiles + works.
- **Rename DONE + committed** (`d69a007`): whole workspace `data-dict*` →
  `dbdict*` (crates, binary `dbdict`, imports), GitHub repo `pjc-wspace/dbdict`,
  `resolve_dict_path` prefers `dbdict.yaml` → legacy `data-dict.yaml` fallback.
- **LICENSE + README credit committed** (`3a85896`). `CLAUDE.md` added.
  `upstream` remote removed; `main` repointed to `origin/main`.
- **3 commits UNPUSHED** on `duckdb-source` (ahead 3). Build + tests green.

## Key Decisions
- **Rich types via `typedef` aliases only** — never invent types the backend
  must create; an alias expands to a backend-native type. Verified DuckDB
  `CREATE TYPE x AS INTEGER|STRUCT(...)|...` supports aliases + compounding, and
  `DESCRIBE`/`typeof` canonicalise for us — so we do zero substitution.
- **Native bundled `duckdb` crate, not shell-out** — post-pivot every shell-out
  reason (portability/upstream/non-duckdb users) evaporated; in-process +
  in-memory fits the round-trip; self-contained binary, no `duckdb` on PATH.
- **Dual-format** — keep coarse/parquet + `types_compatible` for legacy
  `data-dict.yaml`; the rich path is ADDITIVE (do NOT retire coarse types).
- **Stay in Rust, extend the engine** — spec checks + `annotate_snippets`
  diagnostics are the asset; the alias feature is small (DuckDB does the work).
- **Coding style** — training-wheels comments (maintainer learning Rust), plain
  Rust, maintenance > speed. In `CLAUDE.md` + memory `rust-training-wheels-comments`.
- Kept the GitHub fork; added our own MIT LICENSE (upstream declares MIT only via
  Cargo.toml metadata, no LICENSE file / copyright holder).

## Next Steps
- **Decide: push the 3 commits** to `origin/duckdb-source` (still pending).
- **Phase 2 (additive):** `schema.yaml` gains `typedef:` (global + table-scoped)
  and a free-form `type:` (alias or native); grow `model.rs`/`lower.rs`; typedef
  resolution (topo-order + cycle detect). Keep legacy path intact.
- **Phase 3:** rich `validate-meta` via round-trip (build scratch in-memory db
  from the dict, `DESCRIBE`-to-`DESCRIBE` diff → M01/M02/M03); `source.duckdb`.
- **Phase 4:** CLI (`types duckdb`, maybe `expand`), docs, delete the transitional
  shell-out reader; rework `dbdict-duckdb` reader onto the native crate; retire
  coarse `dict_type_for` (rich path).
- **Open item:** S-check ripple — free-form `type:` vs S07/S08/S12–S15 (coupled
  to the coarse enum); revisit in phase 2 (dual-format softens it since coarse
  stays for legacy).

## Relevant Files
- `.claude-work/sessions/20260704-1449-duckdb-spec/goal.md`, `impl.md` — the
  full spec + phased plan (source of truth; impl.md phase-1 marked done).
- `crates/dbdict-duckdb/` — reader crate: `tests/spike_roundtrip.rs` (native,
  proven), `src/lib.rs` (shell-out — transitional), `src/types.rs`
  (`dict_type_for` — transitional), `src/error.rs`, `tests/describe.rs`.
- `crates/dbdict/` — core: `model.rs`, `lower.rs`, `validate_meta.rs`,
  `validate_spec.rs`, `lib.rs` (seam `read_source`/`compare_dataset`).
- `schema.yaml` — `source` object; to gain `typedef:` + free-form `type:`.
- `Cargo.toml` — workspace; `duckdb = { version="~1.10504.0", features=["bundled"] }`.
- `CLAUDE.md`, `LICENSE`.
