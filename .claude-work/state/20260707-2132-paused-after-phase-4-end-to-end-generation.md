---
created: 2026-07-07T21:32:16+12:00
title: paused after phase 4 — end-to-end generation done
tags: [duckdb, rust, design, workflow]
summary: Session 20260707-1410-dummy-data-generator paused between phases 4 and 5. Plan builder (phase 3, commit b120b26) and end-to-end generation with the validate-data oracle (phase 4, commit 46b37eb) are done and green. Next is phase 5, D05 range joins + one-to-one slot-based generation.
---

## Goal
Dummy-data generator session (20260707-1410-dummy-data-generator), paused
cleanly between phases. Six-phase impl.md; phases 1–4 done and committed.
Full plan and record: .claude-work/sessions/20260707-1410-dummy-data-generator/impl.md.

## Current State
- Branch `duckdb-source`, HEAD `46b37eb`, clean tree, `.active` set.
- Phase 3 (commit b120b26): `plan(dict, &opts)` in
  `crates/dbdict-dummy-data/src/plan.rs` — `GenerateOptions` (rows 10,
  table_rows overrides, seed 0, null_fraction 0.25 validated 0..=1),
  Kahn topo order (document-order tie-break), roles IndexedUnique /
  FkDraw{injective} / PlainFill, nullable = !is_required_implied.
  Cardinality analysis mirrors rich.rs D05 positionally; reduces to
  "≥1 join column on each one side is unique-implied". Ten refusal
  paths incl. RangeJoinUnsupported (phase 5 lifts it). 20 unit tests.
- Phase 4 (commit 46b37eb): `generate(dict, &opts)` in
  `crates/dbdict-dummy-data-duckdb/src/generate.rs` — canonical types
  via `instantiate` (typedef aliases → DESCRIBE spellings; untyped
  columns skipped, matching DDL), script = dbdict_ddl::generate + one
  multi-row INSERT per table in plan order; `Generated::write_db(path)`
  refuses existing files, LOADs declared extensions (S19 charset
  re-check) before execute_batch. 7 end-to-end tests incl. two oracle
  fixtures passing `validate_data` with `NativeDuckdb` at Status::Ok.
- Everything green: 33 workspace suites, clippy 0, fmt clean.

## Key Decisions
- Injective FK draws use identity (k = i): `stored_value` resolves
  chains of unique FK columns down to nth(ty, i) with no database
  read-back. A permuted draw would break this — don't "improve" it.
- Deterministic randomness via inline FNV-1a + splitmix64
  `mix(seed, salt, i)` — no rand dependency; byte-identical scripts
  per seed. Seed feeds plain-fill indices, non-injective fk draws,
  AND null placement (so the determinism test compares whole scripts).
- `null_fraction >= 1.0` decided exactly, not via float compare
  (top-end hash values can round to 2^64 and miss `< threshold`).
- write_db never overwrites; deleting first is the CLI's --force
  decision (phase 6).
- Range joins refused outright at plan time rather than "marked
  slot-based in the plan" — the marking machinery lands in phase 5
  with the semantics (recorded as a deviation in impl.md).

## Next Steps
- Phase 5 per impl.md: slot-based generation for range conjuncts —
  "one"-side row k owns a closed interval from monotone `nth` (bounds
  at indices 2k/2k+1), non-overlapping by construction; "many"-side
  probe values strictly inside a chosen slot; handle Gt/Lt open bounds
  and multi-conjunct joins (equality conjuncts pin the slot owner,
  range conjuncts use its interval).
- Lift `RangeJoinUnsupported` in `crates/dbdict-dummy-data/src/plan.rs`
  (check_relationships) and extend the Plan/Role model to carry slot
  info; refuse join shapes outside the scheme (e.g. range bounds that
  are also FK/unique in conflicting ways). `is_orderable` in values.rs
  gates which types may be range bounds.
- Oracle tests: many-to-one range join, one-to-one both directions,
  multi-conjunct mix — all must pass `validate_data`.
- Then phase 6 (CLI subcommand `dummy`, --rows/--seed/--sql/--force,
  e2e CLI tests, docs).

## Relevant Files
- .claude-work/sessions/20260707-1410-dummy-data-generator/{goal.md,impl.md}
- crates/dbdict-dummy-data/src/plan.rs — plan builder; phase 5 edits land here
- crates/dbdict-dummy-data/src/lib.rs — DummyDataError variants
- crates/dbdict-dummy-data-duckdb/src/generate.rs — end-to-end generator
- crates/dbdict-dummy-data-duckdb/src/values.rs — nth/capacity/is_orderable
- crates/dbdict-dummy-data-duckdb/tests/generate.rs — oracle test style
- crates/dbdict/src/rich.rs:383 — D05 check the slots must satisfy
