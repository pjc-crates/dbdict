---
created: 2026-07-08T16:11:04+12:00
title: paused phase 5 — step 2 range plan roles done
tags: [rust, duckdb, design, tdd, workflow]
summary: Session 20260707-1410-dummy-data-generator paused inside phase 5 at the step-2/step-3 boundary. Steps 1 (orientation helper) and 2 (range-join plan roles + refusals, 41 plan tests) are done, UNCOMMITTED but fully green. Next is step 3, backend slot rendering in generate.rs.
---

## Goal
Dummy-data generator session, phase 5 (D05 range joins + one-to-one
slots) of 6. Phases 1–4 + interphase committed (HEAD d67f27f). Phase-5
design is externalized in impl.md — read it (and the step-2 also-notes)
before resuming.

## Current State
- Branch `duckdb-source`, HEAD `d67f27f`. **Working tree is DIRTY and
  green**: phase-5 steps 1 AND 2 are implemented but uncommitted.
  Verified at pause: workspace 387 passed / 0 failed, clippy 0 warnings,
  `cargo fmt --check` clean.
- Step 1 (done, from previous sitting): `JoinExpr::sides/oriented` +
  pub `flip_op` in dbdict::join_expr; `Cardinality::probe_left_directions()`;
  rich.rs and plan.rs both consume the shared helper (review finding 9).
- Step 2 (done this sitting, TDD batches with RED observed each time):
  - crates/dbdict-dummy-data/src/plan.rs: new `Role` variants
    `RangeBound {rel, kind: Lower/Upper}`, `RangeProbe {rel, one_table,
    injective}`, `SlotEqCopy {rel, one_table, one_column}`; `rel` =
    index into dict.relationships (backend's draw salt so a row's roles
    share owner k). `RangeBoundKind` exported from lib.rs.
  - `check_relationships` now returns a claims map `(table, column) →
    Role`; range rels (any non-Eq conjunct) go through
    `claim_range_roles`; pure-eq rels keep the unique-one-side rule.
    Claims are consulted before constraint-derived roles in plan()'s
    column loop; range-role columns are never nullable.
  - Refusals (each with a RED-observed test): `RangeJoinUnsupported`
    repurposed — now means "shape outside the scheme" and carries a
    `reason` string; `RangeColumnIsForeignKey` (ANY range-role column,
    not just bounds); `RangeColumnCannotBeUnique` (non-injective probe,
    and SlotEqCopy unless injective AND source column unique-implied);
    `RangeColumnConflict` (checked claim inserts — identical re-claims
    allowed); `RangeProbeExceedsOneSide` (injective pigeonhole AND
    zero-row one side, applied to both RangeProbe and SlotEqCopy in
    plan()'s main loop via rel-cardinality lookup).
  - tests/plan.rs grew 20 → 41 tests (fixtures: events_windows,
    slot_copy_dict helpers).
  - generate.rs (dbdict-dummy-data-duckdb) has a TEMPORARY arm in
    value_for refusing the three new roles with a descriptive
    `GenerateError::Db { "internal: range-join generation … not
    implemented yet" }` — step 3 replaces it with real rendering.
- impl.md updated: steps 1–2 ticked with also-notes recording the
  stronger-than-planned refusals.

## Key Decisions
- (carried) slot stride is 3: slot k = [nth(3k), nth(3k+2)], probe =
  nth(3k+1) — open and closed bounds uniform, slots disjoint.
- Generation probes the FIRST direction D05 bounds
  (`probe_left_directions()[0]`); one-to-one's reverse direction is
  covered by the injective (identity) owner draw.
- Unique-copy soundness rule (beyond the planned refusal list): a
  unique-implied SlotEqCopy column needs injective draw AND a
  unique-implied source column — copies are only as distinct as their
  source. Bound columns need no unique check (nth injective in row).
- Slot copies draw owners too, so the zero-row/pigeonhole check covers
  SlotEqCopy as well as RangeProbe.
- Unknown join tables/columns are lenient in claim checks (no constraint
  conflict possible) — S02/S03 own those diagnostics; matches the
  existing exact-spelling `dict.table()` lookup convention in plan.rs.

## Next Steps
- Step 3 (crates/dbdict-dummy-data-duckdb/src/generate.rs + values.rs):
  - replace the temporary value_for arm with real rendering:
    RangeBound Lower → nth(3i), Upper → nth(3i+2); RangeProbe →
    owner k = identity if injective else `mix(seed, "range:{rel}", row)
    % one_rows`, value nth(3k+1); SlotEqCopy → same k, then
    stored_value(one_table, one_column, k).
  - IMPORTANT: probe and copy for the same rel must share ONE draw —
    salt by rel index, not by column name (that's why Role carries rel).
  - extend stored_value to recompute PlainFill deterministically (same
    mix formula as value_for's PlainFill arm) so SlotEqCopy can copy
    plain-fill sources.
  - upfront checks before rendering (mirroring UniqueCapacityTooSmall
    loop): is_orderable for bound/probe types, same canonical type
    across one range join's columns, slot capacity 3·one_rows ≤
    capacity(type).
  - NULL interplay: value_for checks nullable BEFORE role — range roles
    have nullable=false from the plan, so no change needed there.
- Oracle tests (tests/generate.rs): many-to-one range join, one-to-one
  both directions, eq+range mix (SlotEqCopy incl. plain-fill source),
  Gt/Lt open bounds — generate → write_db → validate_data == Ok;
  refusal tests per new error path.
- Phase boundary after step 3: /code-review per standing mandate, then
  /ws done (commit includes ALL uncommitted phase-5 work, steps 1–3).

## Relevant Files
- .claude-work/sessions/20260707-1410-dummy-data-generator/impl.md —
  phase-5 design + step 1/2 also-notes (updated this sitting)
- crates/dbdict-dummy-data/src/plan.rs — claim_range_roles, claim(),
  owner-draw row checks, Role/RangeBoundKind (step 2, uncommitted)
- crates/dbdict-dummy-data/src/lib.rs — 4 new + 1 repurposed error
  variants with Display (uncommitted)
- crates/dbdict-dummy-data/tests/plan.rs — 41 tests (uncommitted)
- crates/dbdict-dummy-data-duckdb/src/generate.rs — temporary range arm
  in value_for (line ~330); stored_value; mix() — step 3 lands here
- crates/dbdict-dummy-data-duckdb/src/values.rs — nth/capacity;
  is_orderable lives with DuckType (check exact name at resume)
- crates/dbdict/src/{join_expr.rs,model.rs,rich.rs} — step 1
  (uncommitted)
- .claude-work/notes/20260708-0705-code-review-findings-05941b0-head.md
  — findings 6–8, 10 still open/queued
