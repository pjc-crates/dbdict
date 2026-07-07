---
created: 2026-07-07T18:34:23+12:00
title: paused after phase 2 — typed value generation done
tags: [duckdb, rust, design, workflow]
summary: Session 20260707-1410-dummy-data-generator paused between phases 2 and 3. Extensions declaration (phase 1, commit 8df3c9d) and value generation crates (phase 2, commit 8fc3d20) are done and green. Next is phase 3, the generation plan in dbdict-dummy-data.
---

## Goal
Dummy-data generator session (20260707-1410-dummy-data-generator), paused
cleanly between phases. Six-phase impl.md; phases 1–2 done and committed.
Full plan and record: .claude-work/sessions/20260707-1410-dummy-data-generator/impl.md.

## Current State
- Branch `duckdb-source`, HEAD `8fc3d20`, clean tree, `.active` set.
- Phase 1 (commit 8df3c9d): `duckdb: extensions:` dictionary section —
  model field, lowering, schema-0.2.yaml, S19/S20 spec checks, M10 engine
  check via new defaulted `DuckdbBackend::load_extensions`, LOAD wired
  into instantiate's scratch connections, docs in site/spec.md +
  site/validation.md. The bundled duckdb dep now has the `json` feature.
- Phase 2 (commit 8fc3d20): `crates/dbdict-dummy-data` (stub +
  `DummyDataError`) and `crates/dbdict-dummy-data-duckdb` (`types.rs`
  canonical-type parser → `DuckType`; `values.rs` `nth(ty, i)` SQL
  literals, injective always, monotone for orderable scalars,
  `capacity()` with nested = min of parts). 17 engine round-trip tests +
  7 parser tests; workspace green, clippy/fmt clean.
- Empirical facts pinned: `enable_external_access(false)` blocks external
  extension binaries but statically-linked ones LOAD fine; JSON and plain
  GEOMETRY generate on bundled 1.5.4; `GEOMETRY('EPSG:…')` and INET are
  `Unsupported`; canonical spellings: `TIMESTAMPTZ` → `TIMESTAMP WITH
  TIME ZONE`, `ENUM('a', 'b')` with space, struct field names quoted only
  when needed.

## Key Decisions
- Values are pure functions of (type, index): seed/randomness rides on
  caller-chosen indices (phase 3+), never inside `nth` — preserves the
  injectivity/monotonicity proofs that D02/D03/D04/D05 construction
  depends on.
- Extension availability = "statically linked into the bundled engine";
  LOAD-only policy, `--install-extensions` deliberately future work.
- S19 charset rule ([a-z0-9_]) doubles as LOAD-interpolation safety.

## Next Steps
- Phase 3 per impl.md: `GenerateOptions` (rows global + per-table map,
  seed, NULL fraction), `plan(dict, &opts) -> Result<Plan, DummyDataError>`
  in `crates/dbdict-dummy-data` — refuse legacy, topo-order tables via
  `DataDict::foreign_key_targets` (cycle → error), per-column roles
  (indexed-unique / fk-draw / plain fill), equality-join cardinality
  analysis, range joins marked but refused until phase 5.
- Unit tests with hand-built fixtures (`SourceInfo::for_test`, mirroring
  `crates/dbdict-ddl/tests/generate.rs` helpers).
- Then phase 4 (end-to-end + D01–D05 oracle), 5 (range joins), 6 (CLI).

## Relevant Files
- .claude-work/sessions/20260707-1410-dummy-data-generator/{goal.md,impl.md}
- crates/dbdict-dummy-data/src/lib.rs — plan builder lands here
- crates/dbdict-dummy-data-duckdb/src/{types.rs,values.rs} — done
- crates/dbdict/src/model.rs — DataDict.extensions, foreign_key_targets
- crates/dbdict-ddl/tests/generate.rs — fixture-helper style to mirror
