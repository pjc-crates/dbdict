---
created: 2026-07-08T15:16:36+12:00
title: paused phase 5 — step 1 orientation helper done
tags: [rust, duckdb, design, workflow, adversarial-review]
summary: Session 20260707-1410-dummy-data-generator paused inside phase 5 at the step-1 boundary. Interphase (code-review fixes, commit d67f27f) and phase-5 step 1 (shared JoinExpr::oriented helper, UNCOMMITTED but green) are done. Next is step 2, the range-join plan roles.
---

## Goal
Dummy-data generator session, phase 5 (D05 range joins + one-to-one
slots) of 6. Phases 1–4 committed; an unplanned interphase (code-review
catch-up + capacity fixes) is committed as d67f27f. Full phase-5 design
is externalized in impl.md — read it before resuming.

## Current State
- Branch `duckdb-source`, HEAD `d67f27f`. **Working tree is DIRTY and
  green**: phase-5 step 1 is implemented but uncommitted —
  crates/dbdict/src/{join_expr.rs,model.rs,rich.rs},
  crates/dbdict-dummy-data/src/plan.rs, and impl.md (phase-5 design
  rewrite). `cargo test --workspace` exit 0, clippy 0, fmt clean.
- Step 1 (done): `JoinExpr::sides(probe_left)` + `JoinExpr::oriented(probe_left)`
  + pub `flip_op` in dbdict::join_expr (7 new unit tests);
  `Cardinality::probe_left_directions()` in model.rs; rich.rs
  check_relationships_data and plan.rs check_relationships both consume
  the helper (their inline copies and rich.rs's private flip_op deleted).
  This resolves review finding 9. Bonus fix folded in: plan.rs used
  exact table-name compare where rich.rs used names_eq — helper imposes
  ASCII-case-insensitive on both.
- Review context: 10-finding review note at
  .claude-work/notes/20260708-0705-code-review-findings-05941b0-head.md
  (findings 1–5 fixed in d67f27f; 6–8 and 10 still open; 9 = step 1).
  Standing user mandate: /code-review at every phase boundary.

## Key Decisions
- Slot stride is 3, not the planned 2: slot k = [nth(3k), nth(3k+2)],
  probe = nth(3k+1). With 2k/2k+1 there is NO nth value strictly between
  the bounds, so Gt/Lt would be unsatisfiable. Stride 3 handles open and
  closed bounds uniformly; slots disjoint by monotonicity. Recorded as a
  blockquote in impl.md phase 5.
- Layer split (interphase precedent): plan.rs refuses *structural*
  shapes only; generate.rs refuses *type-level* shapes up front
  (non-orderable bounds, differing canonical types in one range join,
  slot capacity 3·one_rows > capacity).
- A supported range join satisfies D05 by disjoint slots alone — the
  unique-eq-column rule stays for pure-equality joins only.
- Orientation vocabulary is probe-centric everywhere now
  (probe_left_directions + sides + oriented); plan.rs no longer has its
  own inverted one_side_is_left mapping.

## Next Steps
- Step 2 (plan model, crates/dbdict-dummy-data/src/plan.rs): roles
  `RangeBound {rel, Lower/Upper}`, `RangeProbe {rel, one_table,
  injective}`, `SlotEqCopy {rel, one_table, one_column}`; owner k per
  row shared across a row's roles via rel-salted draw, identity when
  one-to-one. Supported shape per probe column: exactly one lower
  (Ge/Gt) + one upper (Le/Lt) against two distinct one-side bound
  columns. Refuse: shape outside scheme, bound column also fk, probe
  column unique-implied on non-injective draw, column claimed by two
  range rels. Range-role columns never NULL. TDD via tests/plan.rs
  fixtures (SourceInfo::for_test helpers already there).
- Step 3 (generate.rs/values.rs): slot index arithmetic for the three
  roles; upfront checks (is_orderable, same canonical type across the
  join's columns, slot capacity); SlotEqCopy stored-value resolution
  (extend stored_value to deterministic PlainFill recompute).
- Oracle tests: many-to-one range join, one-to-one both directions,
  eq+range mix, Gt/Lt open bounds — all pass validate_data; refusal
  tests per new error path.
- Phase boundary: /code-review per standing mandate, then /ws done
  (commit includes the uncommitted step-1 work).

## Relevant Files
- .claude-work/sessions/20260707-1410-dummy-data-generator/impl.md —
  phase 5 design (stride-3 blockquote, layer split, step list)
- crates/dbdict/src/join_expr.rs — new oriented/sides/flip_op + tests
- crates/dbdict/src/model.rs — Cardinality::probe_left_directions
- crates/dbdict/src/rich.rs — check_relationships_data consumes helper
- crates/dbdict-dummy-data/src/plan.rs — step 2 lands here
- crates/dbdict-dummy-data-duckdb/src/{generate.rs,values.rs} — step 3
- .claude-work/notes/20260708-0705-code-review-findings-05941b0-head.md
