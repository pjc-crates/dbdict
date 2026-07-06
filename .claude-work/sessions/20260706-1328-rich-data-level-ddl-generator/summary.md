# summary: rich data level + DDL generator

started: 2026-07-06 13:28
closed: 2026-07-06T16:03:21+12:00

## goal

Make the rich (0.2.0) format useful end to end: give `validate-data` real
data checks against the dictionary's DuckDB database (replacing the
`RichFormatUnsupported` pre-flight), and build the first generator —
`dbdict ddl` — from the public `load_and_lower` model API. Two parked nits
folded in: the `dbdict spec | head` SIGPIPE panic and the case-sensitivity
mismatch between S10 and meta-level table matching.

## what was accomplished

- **phase 1 — nits (e21fa62):** SIGPIPE reset to SIG_DFL at the top of the
  CLI's `main` (unix-only libc call), so piped output dies quietly; S10 now
  detects ASCII-case-insensitive table/column collisions for rich documents
  (legacy stays exact — parquet is case-sensitive — locked by a regression
  test).
- **phase 2 — rich data level (07caa85):** `validate-data` on a rich dict is
  real. `DuckdbBackend` grew `count_nulls` / `count_duplicate_keys`;
  `rich::check_data` runs check_meta then D01 (nulls in required/primary_key
  columns) and D02 (duplicate primary-key values, composite per SQL
  semantics, new code specced in site/validation.md first). Deleted
  `compare_dataset` and the `RichFormatUnsupported` variant.
- **phase 3 — DDL generator (f8e1ee8):** new `crates/dbdict-ddl` with
  `generate(&DataDict) -> Result<String, DdlError>` — `CREATE TYPE` per
  typedef in dependency order, then `CREATE TABLE` per table — plus the
  `dbdict ddl` CLI subcommand, README/site docs, and 23 new red-first tests.
  Refuses typedef shadowing (flat namespace, ASCII-case folded), stalled
  typedefs, legacy dicts, and any script failing its sandboxed
  self-execution check. End-to-end verified: `dbdict ddl` output piped
  through the real `duckdb` CLI built a database that `validate-meta`
  accepts against the same dictionary.

Final state: 248 workspace tests green; clippy + rustfmt clean.

## key decisions

- **Fixpoint over topological sort:** `CREATE TYPE` ordering is discovered
  by executing candidates against a scratch db and recording success order
  (a one-field addition to `create_types_fixpoint`), not by parsing type
  expressions — the same "compare outcomes, don't track dependency edges"
  principle as validation.
- **Shadowing policy v1:** table-scoped typedefs colliding in a flat
  script's single namespace refuse with a clear error listing the sites; no
  renaming scheme until someone asks for one.
- **Seam discipline:** `dbdict-ddl` never names `duckdb::Connection` — the
  backend exposes purpose-built `quote_ident`, `typedef_creation_order`,
  and `execute_and_describe` instead.
- **Narrow named data-level trait methods** (`count_nulls`,
  `count_duplicate_keys`) over a generic query seam — core must not build
  SQL strings; revisit if the trait starts feeling like a query catalogue.
- **S10 folds case for rich only** — folding matches DuckDB identifier
  semantics; legacy parquet names stay exact.
- **Types-only DDL (v1):** constraints (`primary_key`/`required`/`unique`)
  are not translated to SQL clauses — the round-trip yardstick is
  `DESCRIBE`, which is types-only. Natural follow-up if generated schemas
  should enforce what the dictionary declares.

## insights captured

- .claude-work/insights/20260706-1357-format-dependent-validation-and-sigpipe-testing.md
- .claude-work/insights/20260706-1436-rich-data-level-seam-lessons.md
- .claude-work/insights/20260706-1511-ddl-generator-seam-and-fixpoint-ordering.md
