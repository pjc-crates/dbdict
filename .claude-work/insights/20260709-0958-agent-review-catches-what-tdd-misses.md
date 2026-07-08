---
created: 2026-07-09T09:58:35+12:00
title: agent review catches shapes TDD never tested; where a check belongs
tags: [rust, duckdb, tdd, adversarial-review, design]
source: /ws done
---

## agent code review found three real defects rigorous TDD missed

Phase 5 (D05 range joins) was built strict TDD — RED observed before every
step. The end-to-end oracle (generated db → real `validate_data`) passed on
every fixture. Then an 8-angle agent code review found **three** genuine
defects, each confirmed by re-running the exact shape:

- **eq-copy from a non-recomputable source** (`events.b = windows.x` where
  `windows.x` is a slot bound or a non-injective fk) → passed plan + type
  checks, then died mid-generation with a non-actionable `internal:` error.
  Found independently by 3 of the finder agents.
- **one-to-one range join written bounds-first** → falsely refused, because
  `claim_range_roles` tried only `probe_left_directions()[0]`; one-to-one is
  direction-symmetric and needs both directions tried.
- **untyped range bound column** → silently generated a broken database
  (column dropped from DDL, join references a missing column).

The lesson (matches the standing "regular code reviews" mandate): TDD proves
the shapes you *think* to test. Every one of these was a shape I never wrote a
fixture for — an eq-source with an awkward role, a reversed conjunct order, a
missing type. Independent adversarial review enumerates the input space you
didn't. Budget a review at each phase boundary even when the suite is green.

Process note: 6 of 8 finder agents once died mid-run on a model credit limit —
recoverable by re-dispatching the same prompts on a different model. Correctness
angles (line-by-line, removed-behavior, cross-file) are the ones worth paying
for; conventions/altitude can run cheaper.

## a static column constraint cannot express a data property (S06 vs D05)

S06 required a `unique`/`primary_key` column on the "one" side of *every*
join, range joins included. That is category-wrong for range joins: their
at-most-one-match guarantee comes from **disjoint intervals in the data**, not
from any static column constraint. A unique bound is neither necessary
(disjoint non-unique windows satisfy D05) nor sufficient (overlapping
unique-bounded windows still fail D05). Fix: S06 skips any join with a non-`Eq`
conjunct entirely; cardinality for range joins is checked only by D05 at
validate-data time. General rule: if a property can only be decided by looking
at the data, don't fake a spec-level check for it — leave it to the data
validator, or the static check will both miss real violations and block valid
schemas.

## recomputability is the refusal boundary for copy-by-index

`SlotEqCopy` reproduces its source's value purely from an index (no DB
readback). That only works when the source's role is index-recomputable:
index-unique, an *injective* fk draw, or plain fill. A non-injective fk draw or
another slot value is seed-scattered and can't be reconstructed by index — so
the backend refuses those sources up front (`is_recomputable_role`) with an
actionable message, rather than recursing into a `_ => internal error` arm.
When a value is defined by a computation, the set of roles that computation can
invert *is* the supported surface; name it and check it, don't discover it via
a panic. See [[range-claims-map-and-copy-uniqueness]].

## the oracle makes range-join TDD honest; salt by relationship, not column

Every range fixture generates a db and hands it to the real D05 validator —
no bespoke assertion to get subtly wrong. Slot arithmetic: one-side row k owns
`[nth(3k), nth(3k+2)]`, probe = `nth(3k+1)` strictly between, so open and
closed bounds are satisfied uniformly and slots stay disjoint by monotonicity.
Load-bearing detail: a range probe and every eq-copy of the *same* relationship
must draw the *same* slot owner k, so `owner_for` salts `mix()` with the
relationship index (`range:{rel}`), never the column name — keying by column
would let probe and copy disagree about which row matches and fail D05
intermittently. See [[shared-orientation-helper-kills-planner-validator-drift]].
