---
created: 2026-07-19T13:14:13+12:00
title: post-close checkpoint — cleanup-followups shipped
tags: [rust, duckdb, workflow, testing]
summary: The cleanup-followups session is closed and shipped (HEAD 6631195, branch duckdb-source, tree clean bar untracked research/). No work in flight. One open housekeeping question (research/) and the next-piece choice (codegen vs follow-ups) are the only threads.
---

## Goal

No active work session — `cleanup-followups` is complete and closed. This
checkpoint is a resumption pointer for whatever comes next.

## Current State

- Branch `duckdb-source`, HEAD **`6631195`** ("Clear active session tracker").
  Working tree clean except the untracked `research/` dir. No `.claude-work/.active`.
- The 4 cleanup follow-ups + a high-effort code review are all shipped:
  `dfdc184` SLOT_STRIDE · `e1a4f23` legacy test fix · `5bdee96`
  load_lowered_or_exit · `45cf26a` self-contained --sql · `2c07131` review fixes ·
  `302c8d6`/`6631195` close.
- `cargo test --workspace` **415 passed**, clippy 0 warnings, fmt clean.
- Nothing in flight, no uncommitted code.

## Key Decisions

- (carried) `--sql` self-containment scoped to bundled/autoloadable extensions
  (script `LOAD`s, never `INSTALL`s) — verified vs DuckDB docs.
- (carried) Text assertion, not the round-trip, guards the `--sql` fold: json is
  statically linked so no setting forces the explicit `LOAD` — proven by a probe
  that refuted the initial fix. See insight
  `20260719-1304-bundled-extension-autoload-cannot-be-disabled`.
- (carried) Charset-rule dedup (3 crates) deferred to a follow-up.

## Next Steps

Pick one when resuming (none started):
- **Python/Julia codegen** — the next model consumer, the project's stated
  direction. Start with `/ws new`.
- **Recorded follow-ups** (impl.md): hoist the extension-name charset rule to one
  shared validator in `dbdict` core; `--install-extensions` (network INSTALL).
- **Housekeeping (open):** decide on untracked `research/` (Claude Code startup
  notes — scratch, unrelated to dbdict) — gitignore / commit / delete. Awaiting
  user direction.

## Relevant Files

- .claude-work/sessions/20260710-1026-cleanup-followups/summary.md — session record
- .claude-work/sessions/20260710-1026-cleanup-followups/impl.md — phased record +
  review outcomes + follow-ups
- .claude-work/insights/20260719-1304-bundled-extension-autoload-cannot-be-disabled.md
- crates/dbdict-dummy-data-duckdb/src/generate.rs — LOAD fold, SLOT_STRIDE, docs
- crates/dbdict-cli/src/main.rs — load_lowered_or_exit helper
