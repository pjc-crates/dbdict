---
created: 2026-07-09T10:06:15+12:00
title: paused at phase-5/phase-6 boundary — phase 5 committed
tags: [rust, duckdb, tdd, workflow, design]
summary: Dummy-data generator session paused cleanly at the phase-5/phase-6 boundary. Phase 5 (D05 range joins + one-to-one, incl. the review-fix defects and the S06 range relaxation) is DONE and COMMITTED (HEAD 4592f19), working tree clean, workspace fully green. Next is phase 6, the final phase: CLI subcommand + SQL export + docs.
---

## Goal
Dummy-data generator session (`.claude-work/sessions/20260707-1410-dummy-data-generator`),
phase 6 of 6. Phases 1–5 + interphase all committed. Read impl.md for the
full phased plan and the accurate per-phase record (all also-notes).

## Current State
- Branch `duckdb-source`, HEAD **`4592f19`** ("Add D05 range-join
  generation; relax S06 for range joins (phase 5)"). **Working tree is
  CLEAN** — nothing uncommitted.
- Last verify (at /ws done, 2026-07-09): `cargo test --workspace`
  **403 passed / 0 failed**, clippy **0 warnings**, `cargo fmt --check`
  clean. Agent code review done; all findings fixed.
- Phase 5 delivered: range-join value generation (slot arithmetic
  `nth(3k)/nth(3k+1)/nth(3k+2)`, rel-salted owner draws), the shared
  `JoinExpr::sides/oriented/flip_op` helper, plan roles
  RangeBound/RangeProbe/SlotEqCopy + refusals, backend rendering +
  `check_range_types`, and (user-directed) the S06 range-join exemption.
- Three review-found defects were fixed this session (all TDD'd, RED
  first): F1 eq-copy from a non-recomputable source (now refused via
  `is_recomputable_role`), F2 one-to-one range join written bounds-first
  (now tries every probe direction), F4 untyped range column (now refused).

## Key Decisions
- (carried) slot stride 3: one-side row k owns [nth(3k), nth(3k+2)],
  probe = nth(3k+1) strictly between; open and closed bounds uniform.
- (carried) owner draw salts on the RELATIONSHIP INDEX (`range:{rel}`),
  never the column — probe and its eq-copies must agree on the owner row.
- S06 (core validation) now SKIPS any join with a non-Eq conjunct: a
  range join's at-most-one guarantee is a data property D05 checks, not a
  static column constraint (unique bound neither necessary nor sufficient).
  Its orientation moved to the shared helper.
- eq-copy source must have an index-recomputable role (index-unique,
  injective fk, or plain fill); non-injective fk / slot-value sources are
  refused up front rather than hitting stored_value's internal-error arm.

## Next Steps
- **Phase 6 (final): CLI subcommand + SQL export + docs.** Per impl.md:
  - `crates/dbdict-cli/src/main.rs`: add `Command::Dummy { dict, rows,
    table_rows, seed, out, sql, force }` + `run_dummy`, mirroring
    `run_ddl` (see `main.rs` around the `run_ddl` fn): load_and_lower →
    render warnings → `generate` → `write_db` to `--out <file.duckdb>`
    (refuse existing unless `--force`), optional `--sql <file.sql>` export
    (write `Generated.script`).
  - flags: `--rows N` (global default 10), `--rows-table TABLE=N`
    (repeatable → GenerateOptions.table_rows), `--seed N` (default 0).
    NOTE: `Generated.extensions` is private and `write_db` LOADs declared
    extensions itself, so the CLI just calls write_db.
  - e2e tests `crates/dbdict-cli/tests/cli.rs` (`CARGO_BIN_EXE_dbdict` +
    insta): happy path (generate then `validate-data` the output via the
    CLI), sql export, refuse-existing, legacy refusal; update the
    `no_args_lists_all_subcommands` snapshot.
  - docs: README command listing; a site/ page if there's a natural home.
  - `--out` always explicit (never default to the dict's `source.file`);
    `--install-extensions` deliberately NOT built (LOAD-only this session).
- Phase-boundary reminder (standing mandate): run `/code-review` (agent,
  high effort) before `/ws done` for phase 6 too — it caught 3 real bugs
  in phase 5 that TDD missed.
- After phase 6: `/ws close` (writes summary.md).
- Deferred minor polish (recorded in impl.md, not lost): a `SLOT_STRIDE`
  constant for the literal `3`; `refuse`-closure dedups for repeated
  error construction in generate.rs/plan.rs.

## Relevant Files
- .claude-work/sessions/20260707-1410-dummy-data-generator/impl.md —
  phased plan + accurate record (phase 5 marked DONE with all also-notes)
- crates/dbdict-cli/src/main.rs — phase-6 target; mirror `run_ddl`
- crates/dbdict-cli/tests/cli.rs — phase-6 e2e tests
- crates/dbdict-dummy-data-duckdb/src/generate.rs — `generate` →
  `Generated { script, write_db(path) }`; GenerateError variants
- crates/dbdict-dummy-data/src/plan.rs — GenerateOptions (rows,
  table_rows, seed, null_fraction), plan() + Plan::planned_rows
- .claude-work/insights/20260709-0958-agent-review-catches-what-tdd-misses.md
  — this session's insight (review vs TDD; where a check belongs;
  recomputability boundary; oracle + salt-by-rel)
