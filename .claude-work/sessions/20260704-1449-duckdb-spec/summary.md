# summary: DuckDB rich-type data dictionary (typedef aliases)

started: 2026-07-04 14:49
closed: 2026-07-06T13:16:48+12:00

## goal

Pivot the tool from a portable, coarse-typed dictionary to a rich,
DuckDB-native schema tool (`dbdict`): columns typed in DuckDB's own type
domain with a `typedef:` alias layer (global + table-scoped, compounding),
validated against the real database with full type fidelity — while
preserving the legacy `data-dict.yaml` (coarse types + parquet) path
unchanged. Native bundled `duckdb` crate; self-contained binary; extend the
existing Rust engine (source-mapped YAML, S/M checks, annotate-snippets
diagnostics) rather than rewrite it.

## what was accomplished

All four phases done; workspace at 205 tests / 0 failed, clippy + rustfmt
clean, release binary verified self-contained (no `duckdb` on PATH).

- **phase 1 — native crate + round-trip spike (2026-07-04):** mechanism C
  proven: a dictionary instantiated in a scratch in-memory DuckDB
  byte-matches a real table's `DESCRIBE` across the full type zoo (nested
  structs, compounding aliases, fixed/variable arrays, decimals, enums,
  maps). A/B fallbacks dropped.
- **phase 2 — schema + model + `typedef:` parsing (2026-07-05):** `$version`
  discriminator selects the embedded schema (`0.1.0` legacy / `0.2.0` rich);
  rich schema adds `typedef:` (global + table-scoped), free-form column
  `type:`, dictionary-level `source.duckdb.file`, `label:`; lowering carries
  it all with source spans; legacy diagnostics bit-identical.
- **phase 3 — `validate-meta` rewrite (2026-07-05):** core `DuckdbBackend`
  seam (instantiate / read_schema / classify) implemented natively in
  dbdict-duckdb; per-table scratch connections (typedef shadowing), fixpoint
  `CREATE TYPE` (no dependency graph), per-column probes; M06–M09 added;
  S07/S08/S12–S14 reworked as rich compatibility checks; case-insensitive
  name matching; scratch connections sandboxed (external access off, real db
  read-only).
- **phase 4 — CLI + docs + polish (2026-07-06):** `dbdict types duckdb`
  (canonical schema listing) and `dbdict resolve` (typedef → canonical
  expansion, table-context aware); shell-out reader and coarse
  `dict_type_for` deleted; `load_and_lower` public as the model entry point;
  site/spec.md rewritten rich-first with a legacy section; README rewritten
  for dbdict.

Each of phases 2–4 closed with a 3-agent review (correctness / idiom /
tests+docs); all findings actioned same-day. Phase 4's review caught a real
bug: `resolve` contradicted `validate-meta` when a table shadowed a
*dependency* of a global typedef — fixed by emitting table-context entries
whenever a global's expansion differs.

## key decisions

- **Round-trip over own type algebra:** DuckDB expands and canonicalizes
  everything (`CREATE TYPE` + probe + `DESCRIBE`); the tool never parses
  type expressions. Consequences flowed everywhere: fixpoint instead of
  topo-sort, outcome-comparison instead of dependency tracking (phase-4
  review fix), classifier lives with the backend.
- **`$version` as format discriminator;** dual format preserved — legacy
  path untouched down to diagnostic ordering.
- **One dictionary = one DuckDB database** (dict-level `source`), tables
  matched to relations (incl. views) by name, ASCII case-insensitively.
- **Per-table scratch connections** because `CREATE TYPE` names are
  database-global — this is what makes typedef shadowing possible at all.
- **Core stays duckdb-free** via the `DuckdbBackend` trait; the CLI injects
  `NativeDuckdb`; generators will consume the public lowered model.
- **Rich S-checks are compatibility checks, not requirements** — a bare
  DuckDB type carries no measure/id intent, so nothing is required; only
  impossible combinations are rejected.
- **Deferred:** rich *data* level (D01 via duckdb) — honest pre-flight
  stays; fork branding (`$learn_more` URL, `site/CNAME` still
  data-dict.tidyverse.org); pre-existing case-collision nit (two dict tables
  differing only in case both match one relation).

## insights captured

All in `.claude-work/insights/`:

- 20260704-1937-duckdb-reader-type-mapping-and-gating.md — phase-1 reader:
  type mapping + feature gating (both later retired by the pivot)
- 20260705-1004-duckdb-round-trip-create-type.md — CREATE TYPE round-trip
  mechanics
- 20260705-1610-dual-format-validation-delegation-review-lessons.md —
  dual-format + delegation + phase-2 review lessons
- 20260705-2320-duckdb-gotchas-and-trait-seam-testability.md — duckdb
  gotchas, trait-seam testability (phase 3)
- 20260706-1246-typedef-expansion-derived-state-and-review-convergence.md —
  expansion probe reuse, derived-state divergence bug, review convergence
  (phase 4)
