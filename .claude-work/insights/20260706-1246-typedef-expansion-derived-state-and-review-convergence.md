---
created: 2026-07-06T12:46:23+12:00
title: typedef expansion, derived-state divergence, and review convergence
tags: [duckdb, rust, design, testing, adversarial-review]
source: /ws done
---

## expansion probe reuses the round-trip trick
- The expansion probe reuses the round-trip trick from phase 1: we never parse or substitute type expressions ourselves — we `CREATE TYPE` the aliases, create a one-column table typed as the alias, and let `DESCRIBE` hand back DuckDB's canonical spelling. Zero type grammar in our code.
- Table-scoped typedefs force a fresh in-memory connection per table (`CREATE TYPE` names are database-global), which is exactly the machinery `instantiate` already has — the new function shares its `create_types_fixpoint` and effective-set logic instead of duplicating it.

## the resolve command cost almost nothing — seam was cut right
- The `resolve` command cost almost nothing because the hard part already existed: `expand_typedefs` is ~50 lines that reuse `create_types_fixpoint` and the per-table effective-typedef logic from `instantiate`. The refactor that made this clean (extracting `probe_type` and `effective_typedefs`) also *simplified* `instantiate_table` — a good sign the seam was cut in the right place in phase 3.
- Probing an alias as `CREATE TABLE probe (x "money")` relies on DuckDB accepting a quoted identifier in type position — I didn't assume that; the expand tests prove it against the bundled engine, which is the same "let DuckDB decide, pin with tests" discipline the whole rich path uses.

## derived-state divergence and independent-review convergence
- The one real bug (review finding 1) is a classic "derived state diverges from source of truth" failure: `validate-meta` and `resolve` both compute expansions, but from *differently-scoped* scratch databases. The fix makes `resolve` emit a table-scoped entry whenever a global's expansion **differs** in that table's context — comparing outcomes rather than trying to track dependency edges, which is the same "don't parse DuckDB's type grammar" principle that shaped the whole design.
- Note how the three independent lenses converged on the dead `expr` field from different directions (dead code, misleading doc, docs-vs-code mismatch) — agreement between reviewers who can't see each other is a strong signal a finding is real.
