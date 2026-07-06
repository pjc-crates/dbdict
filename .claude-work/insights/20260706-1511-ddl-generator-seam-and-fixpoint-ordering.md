---
created: 2026-07-06T15:11:24+12:00
title: ddl generator seam and fixpoint ordering
tags: [rust, duckdb, design, pattern]
source: /ws done
---

## generator seam: purpose-built functions instead of leaking Connection

The seam design here: `dbdict-ddl` must never name `duckdb::Connection` — that
would drag the duckdb dependency into a third crate. So the backend exposes two
purpose-built functions instead: `typedef_creation_order` (the fixpoint trick,
now recording *success order*, which the existing `create_types_fixpoint`
throws away) and `execute_and_describe` (run a whole script in a sandboxed
scratch db, DESCRIBE everything back). The generator then stays pure string
assembly plus two backend calls.

## declared vs canonical spellings; fixpoint order is a free topological sort

- **The generator emits *declared* spellings, validation compares *canonical*
  ones.** The script says `DECIMAL(18, 4)` and `big`; the database
  canonicalizes to `DECIMAL(18,4)` and `DECIMAL(18,4)[]`. That's why the
  self-check executes the script rather than string-comparing it — the only
  trustworthy equality is `DESCRIBE` output on both sides, the same yardstick
  validate-meta uses.
- **Success order from the fixpoint is a free topological sort.**
  `create_types_fixpoint` already executed `CREATE TYPE`s in
  dependency-respecting rounds; it just discarded the order. One
  `created: Vec<usize>` field turned the validation mechanism into the
  generator's ordering oracle — no type-expression parser needed.
