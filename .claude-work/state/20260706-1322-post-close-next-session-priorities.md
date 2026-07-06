---
created: 2026-07-06T13:22:01+12:00
title: post-close — next-session priorities for dbdict
tags: [duckdb, rust, design, workflow]
summary: duckdb-spec session closed at e85cbdb; no active session. Recommended next: D01 rich data level + first (DDL) generator in one session; fork branding separately. This dump captures the prioritized plan discussed after close.
---

## Goal
Between sessions on `dbdict` (rich DuckDB-native data dictionary). The
20260704-1449-duckdb-spec session is closed — all 4 phases done — and the
next session's scope was just discussed and prioritized.

## Current State
- Branch `duckdb-source`, HEAD `e85cbdb` (session close), clean tree, no
  `.claude-work/.active`.
- Rich (0.2.0) format complete end to end: typedef aliases → round-trip
  `validate-meta` (M01–M09) → `resolve` / `types duckdb` CLI → rich-first
  site/spec.md + dbdict README. 205 tests / 0 failed; clippy + rustfmt
  clean; release binary self-contained (no `duckdb` on PATH).
- `validate-data` on a rich dict still reports the honest
  `RichFormatUnsupported` pre-flight — the one user-visible gap.

## Key Decisions
(recommendation given post-close, not yet user-confirmed)
- Next session should combine **D01 rich data level** (small: seam +
  source resolution + table matching all exist from phase 3; one query per
  required/primary_key column + tests) with the **first generator — DDL**
  (nearly free: the dict already round-trips through CREATE TYPE/CREATE
  TABLE, so the scratch DDL *is* the generator; pressure-tests the public
  `load_and_lower` model API before more code accretes).
- Fork branding ($learn_more URL, site/CNAME, upstream-flavoured
  site/index.md) is a separate, cheap, decision-heavy session — blocks only
  site publishing.
- Nits to fold into whichever session touches the area: `dbdict spec |
  head` SIGPIPE panic (fix = reset SIGPIPE in main); case-insensitive
  table-name collision (S10 exact vs meta-level ASCII-folded matching).

## Next Steps
- `/ws new` for the chosen scope (recommended: "rich data level + DDL
  generator").
- If D01: extend `validate_data` past the `RichFormatUnsupported` gate in
  crates/dbdict/src/lib.rs (`compare_dataset`); nulls check per
  required/primary_key column against the real db via the backend seam
  (needs a new trait method or a data-level seam in dbdict::rich).
- If DDL generator: new crate (e.g. crates/dbdict-ddl) consuming
  `dbdict::load_and_lower`'s model per the architecture rule — generators
  never touch YAML or the CLI; wire as a CLI subcommand.
- Fresh session: `/state load` this file, then `/ws new`.

## Relevant Files
- .claude-work/sessions/20260704-1449-duckdb-spec/summary.md — closed
  session record; impl.md there has per-phase decisions + review logs
- .claude-work/state/20260706-1316-session-closed-all-phases-done.md —
  close-time dump (fuller file map)
- crates/dbdict/src/lib.rs — `compare_dataset` / `RichFormatUnsupported`
  gate (D01 entry point); `load_and_lower` (generator entry point)
- crates/dbdict/src/rich.rs — `DuckdbBackend` seam to extend for data-level
- crates/dbdict-duckdb/src/native.rs — scratch/instantiate mechanics a DDL
  generator would reuse
