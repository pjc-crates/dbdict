---
created: 2026-07-05T16:10:18+12:00
title: dual-format validation, delegation, and review lessons
tags: [rust, duckdb, design, gotcha, adversarial-review]
source: /ws done
---

## format discrimination must precede schema validation
The embedded schema is `closed: true` everywhere — a `typedef:` key or a
`type: STRUCT(...)` value is rejected *structurally*, before any S-check
runs. And S07's else-branch actively demands `examples` for any
unrecognized type name, so rich types don't just slip through the coarse
checks, they trip them. Dual-format support therefore can't be patched in
after validation: the two formats must be discriminated *before* schema
validation. The `$version` peek in `load()` is the single switch point.

## dict-level source widens what validation can see
A dict-level database source (one dict = one database) changes what
validation *can see*. With per-table sources, validate-meta can only check
tables the dict mentions — an undocumented table in the db is invisible.
With a db-level source, the tool opens one database and can diff the table
sets both ways: dict-only tables and undocumented db tables both become
reportable. The legacy path structurally couldn't have this (one parquet
file = one table). Also: one connection opened once, not per-table.

## the format marker lives on the lowered model
Lowering is the last time the raw YAML document is visible — everything
downstream (spec checks now, the round-trip differ later, generators
after that) reads the model only, so `Format` belongs on `DataDict`, not
re-derived at each check site. Corollary discovered via review: gate
format-specific checks *in place*, not by hoisting calls — two problems at
the same span tie in the stable source-order sort, so rendered order is
push order, and moving a call reorders legacy diagnostics (a real,
test-pinnable regression).

## delegation beat ownership twice
Two planned phase-2 work items dissolved on contact with reality:
(1) duplicate typedef names were already rejected a layer down — the
schema validator flags duplicate mapping keys structurally, so the custom
S-check written for it was unreachable and got deleted; (2) typedef
dependency graphs can't be reliably extracted from type expressions
textually (a struct *field name* is indistinguishable from a type
reference without duckdb's grammar — `STRUCT(trade VARCHAR)`), so
cycle/order resolution moved to a `CREATE TYPE` fixpoint-retry in the
scratch db, where duckdb's parser is the authority. Net phase code stayed
small; the tests pinning the format boundary are the real deliverable.

## diverse-lens review: each critical was invisible to the other lenses
Three parallel reviewers with distinct lenses (correctness / idiom /
test-plan) each produced exactly one Critical — and each was invisible to
the other two lenses: a diagnostic-order regression needed
behavior-vs-HEAD archaeology, stale copied schema comments needed
convention auditing, and a coverage claim needed plan-vs-test
cross-walking. Also: empirically verify review claims before accepting
them — the "gate has zero coverage, deletable without failures" Critical
failed reproduction (3 tests break without the gate), shrinking it to a
real-but-minor gap ("S15-still-fires is unpinned").
