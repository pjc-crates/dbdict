---
created: 2026-07-06T13:57:19+12:00
title: format-dependent validation and sigpipe testing
tags: [rust, duckdb, gotcha, design, workflow]
source: /ws done
---

## outcome comparison over representation comparison (DDL generator plan)
- The DDL generator's verify step reuses the project's core trick: instead of asserting on generated SQL text (brittle), execute the DDL in a scratch DuckDB and diff `DESCRIBE` output — comparing outcomes, not representations, exactly like `validate-meta` does.
- D02 is scoped rich-only. The legacy path is deliberately frozen (fork philosophy: preserved, not extended), which keeps the parquet backend from needing a uniqueness query it was never designed for.

## flat DDL scripts: ordering and shadowing wrinkles
- The interesting problem in DDL generation isn't generating `CREATE` statements — it's **ordering** them. Typedefs can reference each other, and a flat script needs dependency order. Instead of writing a topological sorter over type expressions (which means parsing DuckDB's type grammar — something this project deliberately refuses to own), the plan reuses `create_types_fixpoint`'s trick: execute candidates against a scratch db, emit in the order that succeeded.
- Table-scoped typedef **shadowing** is the second wrinkle: per-table scratch connections gave each table its own type namespace, but a flat DDL script has exactly one. The v1 answer is to refuse with a clear error rather than invent a renaming scheme.

## uniqueness rules must follow the storage backend's case semantics
- The S10 fold had to be **format-dependent**, not global: rich names live in DuckDB (case-insensitive identifiers), legacy names live in parquet (case-sensitive), so one S10 rule can't serve both. The existing `coarse_types = dict.format == Format::Legacy` gate in `check_spec` was the precedent for branching validation on format in place.
- The error message changes only for the rich path ("DuckDB identifiers are case-insensitive") — snapshot tests make this kind of user-facing wording change reviewable as a diff rather than a hidden side effect.

## testing SIGPIPE deterministically; exit 141 is correct
- The SIGPIPE test's determinism trick: with only 22 KB of output (under the 64 KiB pipe buffer), `| head` reproduces the panic only racily. Closing the read end **immediately after spawn** — before the child finishes startup — guarantees its first write hits a closed pipe, making the test reliable.
- Exit code 141 = 128 + 13 (SIGPIPE). Shells treat signal-death in a pipeline as normal; only `pipefail` even surfaces it. That's why dying quietly is the correct unix behaviour rather than catching EPIPE and exiting 0.
