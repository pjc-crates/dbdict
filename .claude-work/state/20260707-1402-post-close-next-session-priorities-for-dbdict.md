---
created: 2026-07-07T14:02:35+12:00
title: post-close — next-session priorities for dbdict
tags: [duckdb, rust, design, workflow]
summary: Between sessions at bf09145; no active session. Two sessions closed today in this conversation (D05 cardinality verification, fork branding). Recommended next: dummy-data generator in a fresh session; site publishing decision and Python/Julia codegen behind it. This dump captures the prioritized plan discussed after close.
---

## Goal
Between sessions on dbdict. The 20260707-1034-fork-branding session is
closed (both phases done) and next-session options were just discussed
and prioritized. This conversation also closed 20260707-0901-d05
earlier — it has run two full sessions, so the next session should
start fresh.

## Current State
- Branch `duckdb-source`, HEAD `bf09145` (fork-branding close), clean
  tree, no `.claude-work/.active`.
- Rich data level complete: D01–D05 cover every declared constraint
  (`required`, `primary_key`, `unique`, `foreign_key`, `cardinality`);
  the declared-constraint queue is exhausted (`conflicts` explicitly
  out of scope by decision).
- Fork branding complete: canonical `$learn_more` =
  https://github.com/pjc-wspace/dbdict; site rebranded (no CNAME, no
  analytics, dbdict-first index.md with Lineage section);
  .claude/claude.md describes the fork accurately. Remaining upstream
  references are attribution only (README, LICENSE, root CLAUDE.md,
  index.md Lineage).
- 290 workspace tests green; clippy + fmt clean.
- Generators so far: DDL (crates/dbdict-ddl). quarto not installed on
  this machine.

## Key Decisions
(recommendation given post-close, not yet user-confirmed)
- Next session: **dummy-data generator** — first consumer that must
  *interpret* the type system (structs, enums, decimals, arrays)
  rather than round-trip it; D01–D05 are a built-in oracle (generated
  data should pass validate-data by construction). Satisfying
  many-to-one range joins by construction is the interesting design
  problem — worth a proper /ws plan.
- Site publishing decision is small, decision-heavy, unblocked; wants
  quarto installed for a local render check. Do whenever.
- Python/Julia codegen after dummy data — same resolved model; dummy
  data shakes out model-API gaps first.

## Next Steps
- Fresh session (two sessions closed in this conversation): restart,
  `/state load` this file, then `/ws new` for the chosen scope.
- If dummy-data generator: new crate (e.g. crates/dbdict-dummy)
  consuming the core model per the architecture rule (generators never
  touch YAML or the CLI); wire as a CLI subcommand; plan the
  constraint-satisfaction order (types → uniqueness → fk targets →
  cardinality bounds).
- If publishing: enable GitHub Pages, confirm/adjust site-url in
  site/_quarto.yml (currently the inferred
  https://pjc-wspace.github.io/dbdict/), install quarto, render
  locally.

## Relevant Files
- .claude-work/state/20260707-1359-session-closed-fork-branding-complete.md
  — close-time dump (fuller file map)
- .claude-work/sessions/20260707-1034-fork-branding/summary.md
- .claude-work/sessions/20260707-0901-d05-cardinality-verification/summary.md
- CLAUDE.md (root) — architecture rule for generator crates
- crates/dbdict/src/lib.rs — load_and_lower (generator entry point)
- crates/dbdict-ddl/ — the existing generator to mirror structurally
