---
created: 2026-07-06T14:43:01+12:00
title: paused before phase 3 — DDL generator
tags: [rust, duckdb, design, workflow]
summary: Session 20260706-1328 paused between phases. Phases 1 (SIGPIPE + rich S10 fold) and 2 (rich data level D01/D02) done and committed (e21fa62, 07caa85). Phase 3 (dbdict-ddl crate + dbdict ddl subcommand) not started; design decisions already annotated in impl.md.
---

## Goal
Work session 20260706-1328-rich-data-level-ddl-generator: rich data level
(D01 + D02) plus the first generator (DDL), with two nits folded in
(SIGPIPE, S10 case folding). Session is PAUSED mid-way — phases 1–2 of 3
done, phase 3 not started.

## Current State
- Branch `duckdb-source`, HEAD `07caa85`, clean tree, `.active` points at
  the session.
- Phase 1 DONE (e21fa62): SIGPIPE reset in cli main (`reset_sigpipe()`,
  unix-only libc dep); S10 folds ASCII case for rich documents only
  (legacy/parquet stays exact, regression-locked); validation.md updated.
- Phase 2 DONE (07caa85): rich `validate-data` is real — `DuckdbBackend`
  gained `count_nulls`/`count_duplicate_keys`, `rich::check_data` runs
  check_meta then D01/D02 queries, `validate_data` takes the backend seam,
  `compare_dataset`/`RichFormatUnsupported` deleted, D02 specced in
  validation.md. 225 tests / 0 failed; clippy + fmt clean.
- Phase 3 NOT STARTED: `crates/dbdict-ddl` + `dbdict ddl` subcommand.

## Key Decisions
(see impl.md blockquotes for full rationale)
- DDL generator: new crate depends on dbdict (model via `load_and_lower`)
  AND dbdict-duckdb — CREATE TYPE ordering discovered by executing against
  a scratch db (reuse of the fixpoint trick), not by parsing type
  expressions.
- Shadowing policy v1: table-scoped typedefs that collide in a flat
  script's single namespace → refuse with a clear error, no renaming
  scheme.
- Round-trip self-check: execute generated DDL in a fresh in-memory
  duckdb and diff canonical DESCRIBE against the dict's own instantiation
  (the validate-meta trick).
- Data-level trait methods are narrow and named (no generic query seam);
  queries use db-side name spellings, problems locate at dict spans.

## Next Steps
- Resume with phase 3, TDD (red first, as phases 1–2):
  1. new workspace member `crates/dbdict-ddl` exposing
     `generate(dict: &DataDict) -> Result<String, ...>`
  2. round-trip integration tests (typedef chains, structs/enums/decimals/
     arrays, shadowing refusal)
  3. wire `dbdict ddl <dict>` flat subcommand in crates/dbdict-cli
     (stdout script; errors to stderr, nonzero exit) + e2e snapshot;
     README + site docs for the new command
- Useful mechanics to reuse: `create_types_fixpoint`, `quote_ident`,
  `open_scratch` in crates/dbdict-duckdb/src/native.rs; per-table scratch
  instantiation shows where shadowing detection has to look.
- Then `/ws done`, and `/ws close` (all phases complete).

## Relevant Files
- .claude-work/sessions/20260706-1328-rich-data-level-ddl-generator/
  {goal,impl}.md — impl.md has per-phase records + decision blockquotes
- crates/dbdict/src/rich.rs — DuckdbBackend trait + check_meta/check_data
- crates/dbdict/src/lib.rs — load_and_lower (the generator's entry point)
- crates/dbdict-duckdb/src/native.rs — fixpoint/scratch/quote mechanics
- crates/dbdict-cli/src/main.rs — subcommand wiring pattern
- site/validation.md — S10/D01/D02 text updated this session
