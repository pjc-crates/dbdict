---
created: 2026-07-05T16:14:37+12:00
title: phase 2 rich format done, reviewed, committed
tags: [rust, duckdb, design, workflow, adversarial-review]
summary: Phase 2 (rich 0.2.0 format — $version schema selection, typedef, dict-level source) implemented TDD-style, reviewed by 3 independent agents, all findings fixed, committed. Next is phase 3 (round-trip validate-meta). 5 commits unpushed.
---

## Goal
Build `dbdict` — rich DuckDB-native data dictionary (fork of tidyverse
data-dict). `typedef:` alias layer over native duckdb types; validation
round-trips the dict through an in-memory duckdb and diffs
DESCRIBE-to-DESCRIBE (mechanism C, proven in phase 1). Dual-format: legacy
`data-dict.yaml` (0.1.0, coarse+parquet) preserved exactly; rich
`dbdict.yaml` (0.2.0) added alongside.

## Current State
- **Phase 2 DONE** (2026-07-05T16:10, commit `122664b`; fmt-only fallout
  split into `25b6080`). 163 tests / 0 failed, clippy clean, tree clean.
- Rich format live at spec level: `load()` peeks `$version` → picks
  embedded schema (`schema.yaml` legacy / `schema-0.2.yaml` rich; any other
  *present* value → span-located unsupported-version error, incl. unquoted
  floats like `0.2`). Global + table-scoped `typedef:`, free-form `type:`,
  top-level `source.duckdb.file`, `label:` on tables+columns. Model carries
  `Format`, `Typedef`, `DictSource`, labels with spans.
- Coarse checks S07/S08/S12–S14 gated off **in place** for rich docs
  (same-span problems keep push order — hoisting reorders legacy
  diagnostics); S15 still runs. S18 = non-string typedef name.
  Rich docs at validate-meta/data → one "not yet supported" preflight
  (`ProblemKind::RichFormatUnsupported`), not N misleading M04s.
- **3-agent review done** (correctness / idiom / tests-plan): all findings
  fixed same day or declined with reasons; recorded in impl.md. One review
  claim disproven by reproduction (gate coverage).
- **5 commits ahead of origin/duckdb-source, UNPUSHED** — push decision
  still open (asked twice, unanswered).

## Key Decisions
- `$version` is the format discriminator; discrimination must happen
  BEFORE schema validation (closed schemas + S07 else-branch trip on rich).
- Top-level `source:` (one dict = one database) — enables table-set diff
  both ways in phase 3; dict table name == db relation name (no override).
- Typedef cycles/ordering NOT checked at spec level: textual dependency
  extraction can't tell a struct field name from a type reference
  (`STRUCT(trade VARCHAR)`), so resolution is a `CREATE TYPE`
  fixpoint-retry in the scratch db (phase 3). Spec level keeps only what's
  reliable; duplicate keys already rejected by the schema validator.
- S07/S08/S12–S14 rework for duckdb types deferred to phase 3 (needs the
  canonicalized-type classifier via the scratch-db seam; core stays pure —
  trait injected from dbdict-duckdb).

## Next Steps
- **Decide: push** the 5 commits to origin/duckdb-source.
- **Phase 3** (impl.md has full step list): scratch in-memory db seam
  (fixpoint CREATE TYPE + CREATE TABLE), DESCRIBE-to-DESCRIBE diff
  reframing M01/M02/M03, table-set diff, resolve `source.duckdb.file`
  relative to dict, rework S07/S08/S12–S14 via type classifier, replace
  the transitional RichFormatUnsupported preflight, retire
  `types_compatible` on the rich path.
- Phase 4: CLI (`types duckdb`, maybe `expand`), docs, delete transitional
  shell-out reader in dbdict-duckdb.

## Relevant Files
- `.claude-work/sessions/20260704-1449-duckdb-spec/{goal,impl}.md` — spec +
  phased plan with decision blockquotes (source of truth)
- `crates/dbdict/src/validate_spec.rs` — schema selection (`select_schema`,
  `version_text`), gate in `check_spec`, S18 via lowering
- `crates/dbdict/src/{model,lower}.rs` — Format/Typedef/DictSource/labels
- `crates/dbdict/src/lib.rs` — compare_dataset rich preflight (replace in
  phase 3); `crates/dbdict/src/problem.rs` — RichFormatUnsupported
- `crates/dbdict/tests/rich.rs` — all rich-format tests; `tests/common/mod.rs`
  now owns `diagnostics`
- `schema.yaml` + `schema-0.2.yaml` — twins with reciprocal drift banners
- `crates/dbdict-duckdb/tests/spike_roundtrip.rs` — phase-1 proof, feeds
  phase 3's seam
