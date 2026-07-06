# implementation: DuckDB rich-type data dictionary (typedef aliases)

> **Re-planned 2026-07-04** after the pivot to rich DuckDB-native types + the
> native bundled `duckdb` crate. Supersedes the original coarse / shell-out /
> feature-gate plan. The committed phase-1 shell-out reader (`a01ba1b`) is being
> **reworked** onto the native crate; its type-string/DESCRIBE knowledge and the
> `dict_type_for` test intent carry over, but the coarse mapping itself retires.

Branch: `duckdb-source`. Native **bundled** `duckdb` crate (`~1.10504.x` = DuckDB
1.5.4). Self-contained binary; no runtime `duckdb`-on-PATH dependency.

## phases

### phase 1: native crate + round-trip spike
Prove mechanism **C** (round-trip) before building the feature on it.
- [x] add bundled `duckdb` dep (workspace + `data-dict-duckdb`) — first C++ build
      running (bg `bawi1y9jr`).
- [ ] **spike test** (in-process): `Connection::open_in_memory()`, `CREATE TYPE`
      (incl. compounding: alias-in-alias) + `CREATE TABLE`, `DESCRIBE` the scratch
      tables, and assert the result **byte-matches** an equivalent real table's
      `DESCRIBE` across the type zoo: nested `STRUCT`, `LIST`/`MAP`/`UNION`,
      fixed `T[N]` vs variable `T[]`, `DECIMAL(p,s)`, `ENUM` (field-order / case /
      whitespace).
- [ ] spike: malformed typedef, **cyclic** typedef, unknown alias → do they error
      usefully on instantiation? Record where diagnostics come from.
- [ ] rework the reader onto the native crate: `describe(conn, table) ->
      Vec<(name, native_type)>` (drop `std::process::Command`); **retire the
      coarse `dict_type_for`**; port/trim the tests.
- **verify:** `cargo test -p data-dict-duckdb` green; the spike proves C (or a
      fallback A/B is chosen and the reason recorded here).
- **SPIKE RESULT (2026-07-04): C confirmed.** Full type zoo byte-matched
      in-process (struct+compounding, fixed/var arrays, decimal, enum, map);
      enums fully expand; unknown/forward typedefs error. Native crate API clean.
      A/B dropped.

### phase 2: schema + model + `typedef:` parsing — DONE 2026-07-05T16:10:18+12:00

> **decisions (2026-07-05, discussed with user):**
> - **format discriminator: `$version`** — `0.1.0` = legacy (schema + checks
>   untouched), `0.2.0` = rich. `load()` peeks `$version` and selects the
>   matching embedded schema; unknown version → clear pre-flight error.
> - **top-level `source:`** (sibling of `tables:`) — one dict = one database;
>   `source.duckdb: { file }`, path relative to the dict (absolute ok). the
>   per-table `source.parquet` stays legacy-only. dict-level source enables
>   table-set diffing both ways (undocumented db tables become visible).
> - **dict table name == db relation name** — no per-table override for now
>   (can be added later without breakage).
> - **`label:`** (optional display name) added to tables AND columns (rich).
> - descriptive keys (`values`/`range`/`examples`/`units`/`time_zone`) **stay**
>   in the rich schema; S07/S08/S12–S15 are **reworked** for duckdb types —
>   but that rework moves to phase 3 (it classifies *canonicalized* types, so
>   it depends on the scratch-db seam built there; phase 2 stays duckdb-free).
> - typedef resolution: **fixpoint via duckdb, no dependency graph** (decided
>   2026-07-05, supersedes "own the topo-sort"). extracting dependencies from
>   type expressions can't reliably tell a type reference from a struct
>   *field name* (`STRUCT(trade VARCHAR)`) without hand-rolling duckdb's type
>   grammar — phantom cycles would reject valid dicts. instead phase 3 runs
>   `CREATE TYPE` retry-until-stall in the scratch db: real dependencies
>   resolve themselves, the stalled leftovers are exactly the cyclic/unknown
>   group, duckdb's error names the problem, our spans locate it. phase 2
>   keeps only **duplicate-name detection** (pure name comparison, reliable;
>   the yaml parser preserves duplicate mapping keys — probed). global-vs-table
>   same name is shadowing, by design, not a duplicate.

- [x] `$version` peek + schema selection in `load()`: `0.1.0` → embedded
      legacy schema, `0.2.0` → embedded rich schema, other/missing → error.
      - also (review): any *present* non-string `$version` (`0.2`, `2`, null)
        takes the unsupported-version path too — only a truly absent key
        falls to the legacy schema's required-key error.
- [x] rich schema (`schema-0.2.yaml`): top-level **`typedef:`** (map `name →
      type-expression string`) + table-scoped `typedef:`; column `type:` as
      **free-form string** (alias name or native DuckDB type expr); top-level
      `source:` with `duckdb: { file }`; `label:` on tables + columns;
      descriptive keys kept. reciprocal drift banners in both schema files
      (shared subschemas are duplicated — edit both).
- [x] `model.rs` / `lower.rs`: dict format marker (legacy | rich); typedefs
      (global + per-table, table shadows global); column type-as-string
      (already is); top-level source; `label`; all carrying source spans.
      - also (review): S07/S08/S12–S14 gated *in place* for rich docs so
        legacy diagnostic order is bit-identical (two problems at one span
        keep push order); S15 still runs for rich.
      - also (review): **S18** — a non-string typedef *name* (`123:`) is a
        spec error, not a silent drop (schema constrains values only).
      - also (review): rich docs at validate-meta/data get one honest
        "not yet supported at this level" pre-flight instead of a misleading
        M04 per table, until phase 3 wires the duckdb source in.
- [x] typedef **duplicate-name detection** — resolved without an S-check: the
      schema validator already rejects duplicate mapping keys structurally
      (`Duplicate key 'money'`, span on the second definition). a duplicate-
      name S-check was written, found unreachable, and removed (S18 was later
      reused for non-string typedef names); the guarantee is pinned by tests.
      shadowing (table redefines a global name) stays legal. ordering/cycles/
      unknown-alias moved to phase 3 (fixpoint, see above).
- **verify:** `cargo test`; legacy fixtures validate unchanged (incl. S14/S15
      same-span diagnostic order); lowering tests (global + scoped typedef,
      shadowing legal, duplicate → error, non-string name → S18); a rich dict
      with `typedef:` + top-level source parses. — cycles are *not* a phase-2
      error (fixpoint decision above moved them to phase 3).
- **review (2026-07-05):** three independent agents (correctness / idiom /
      tests-plan); all findings fixed or explicitly declined this same day.

### phase 3: `validate-meta` rewrite (full fidelity, round-trip) — DONE 2026-07-05T23:20:00+12:00

> **decisions (2026-07-05, made while implementing — review at phase end):**
> - **seam shape:** core defines `rich::DuckdbBackend` (trait: `instantiate`,
>   `read_schema`, `classify`) + plain data types; `dbdict-duckdb` depends on
>   `dbdict` and implements it (`NativeDuckdb`); the CLI passes it in.
>   `validate_meta` gained a third parameter (`&dyn DuckdbBackend`);
>   `validate_data`'s signature is unchanged (rich data level still a
>   pre-flight). core stays free of the bundled duckdb build.
> - **one scratch connection per table** — `CREATE TYPE` names are
>   database-global, so table-scoped shadowing can't live in one shared
>   scratch db. globals fixpoint once in their own connection (stage 1, so a
>   broken global reports once, not per table); each table then gets a fresh
>   connection with its *effective* typedefs (globals minus shadowed, plus
>   scoped). global failures in a table's stage are dropped as echoes.
> - **probe-per-column, not one CREATE TABLE** — each typed column is created
>   as its own single-column table and DESCRIBEd. canonicalization is
>   per-column, so this equals the whole-table DESCRIBE, and a bad column
>   can't take down its table's expected side (no combined-create failure
>   mode to mis-attribute).
> - **new codes:** M06 dict table missing from db (error); M07 undocumented
>   db table/view (warning, mirrors M03, skipped under `--table`); M08
>   rejected typedef (error, duckdb's reason at the typedef span); M09
>   rejected column type (error, at the `type:` span). M04/M05 reused at
>   dictionary level (no per-table source in rich). codes documented in
>   site/validation.md.
> - **views count as relations** on the real side (a dict table may be backed
>   by a view); relations read from `information_schema.tables`, `main`
>   schema, alphabetical.
> - **S07/S08/S12–S14 rich semantics are compatibility checks, not
>   requirements** — the coarse qualifiers (`number(quantity)` vs
>   `number(id)`) carried intent a bare duckdb type doesn't, so nothing can
>   be *required*. rejected: `range` on unorderable types (ENUM/BOOLEAN/
>   composite/other), any representation on BOOLEAN, `units` off numerics,
>   `time_zone` off timestamps; S12/S13 check range bounds per category
>   (naive datetimes for TIMESTAMP, offset-carrying for TIMESTAMP WITH TIME
>   ZONE). they run at the meta level (canonicalization needs the scratch
>   db) but keep their S codes — rule identity over level. `values` on a
>   VARCHAR column is legal (categorical columns without a db-side ENUM).
> - **dict-side checks run before source problems** — instantiation failures
>   (M08/M09) and the descriptive-key checks report even when the database
>   is missing/unreadable; M04 (no source at all) still returns early.
> - `instantiate` panics only if an in-memory duckdb can't be created at all
>   (resource exhaustion — no dictionary input reaches it); accepted rather
>   than threading a Result through the seam.

- [x] seam: scratch in-memory db from the dict (`CREATE TYPE` + `CREATE
      TABLE`), `DESCRIBE` → **expected**; open the real duckdb db (read-only,
      native crate) → **actual**; diff. (`dbdict::rich` + `dbdict-duckdb::
      {instantiate, read_schema, NativeDuckdb}`)
- [x] resolve `source.duckdb.file` **relative to the dict** (absolute as-is);
      replaced the transitional pre-flight in the *meta* path — `validate-data`
      keeps an honest data-level pre-flight until the rich data level exists.
- [x] table-set diff: M06 (dict table missing from db, error) and M07
      (undocumented db table/view, warning; skipped under `--table`).
- [x] rework S07/S08/S12–S14 for rich mode via `TypeCategory` classifier
      (trait method, implemented in dbdict-duckdb, tested against real
      DESCRIBE spellings); S15 already ran for rich. legacy unchanged.
- [x] M01/M02/M03 reframed as the `(name, canonical_type)` diff — M01 is an
      exact string compare of canonical types, problems located at dict spans.
- [x] `types_compatible` never runs on the rich path (it stays for legacy
      parquet; the coarse `dict_type_for` mapping in dbdict-duckdb retires
      with the shell-out reader in phase 4).
- [x] dict coherence via **fixpoint instantiation**: retry-until-stall
      `CREATE TYPE`; stalled leftovers (cyclic/unknown) report duckdb's error
      at the typedef span (M08); rejected column types at their span (M09).
- **verify (end-to-end): PASSED 2026-07-05** — `dbdict-duckdb/tests/
      e2e_validate_meta.rs`: clean match → ok; struct field type wrong → M01
      with the exact `STRUCT(...)` diff both sides; dropped documented col →
      M02; undocumented db col → M03; cyclic typedefs → M08 pair with
      duckdb's reason, span-located; missing db file → M05 at the source
      entry. CLI round-trip test passes (`validate-meta` exit 0 on a clean
      rich dict + real db). workspace: 202 tests, 0 failed; clippy clean;
      rustfmt clean.

> **3-agent review (2026-07-05, correctness / idiom / tests-plan) — all
> findings actioned, verified firsthand against the bundled duckdb before
> fixing. workspace now 210 tests, 0 failed; clippy + rustfmt clean.**
> - **BUG (correctness, verified end-to-end): case-sensitivity.** duckdb
>   identifiers are case-insensitive but case-preserving in DESCRIBE, so a
>   lowercase dict vs a CamelCase db produced spurious M02/M03/M06/M07. FIXED:
>   `rich::names_eq` (ASCII case-fold) on every dict↔db name match (table
>   match, M06, M07, M03, M01 actual-side); the scratch `expected` side and
>   the `--table` filter stay exact (same-source / user-arg). Type-string
>   comparison stays exact (canonical types are already normalised). Pinned by
>   e2e `identifier_case_differences_still_match`.
> - **SECURITY/correctness (verified: an ATTACH in a type expr created a file
>   on disk).** duckdb's `execute` runs *all* statements in a string, not one
>   — the old comment claiming otherwise was false. A dictionary is untrusted
>   shared input. FIXED: scratch connections open with
>   `enable_external_access(false)` (blocks ATTACH/COPY/read_csv; normal types
>   unaffected); comment corrected to state the real safety basis (throwaway
>   in-memory + external access off + real db read-only). Pinned by
>   `type_expression_cannot_reach_the_filesystem`.
> - **correctness: phantom columns.** a malformed type with a top-level comma
>   made `probe` multi-column, leaking phantom columns into the expected side
>   (cross-column false-fail / false-pass). FIXED: the per-column probe now
>   requires exactly one DESCRIBE row, else M09. Pinned by
>   `malformed_type_with_top_level_comma_is_a_column_failure`.
> - **idiom/dead-generality: `Instantiated.tables` was `Vec<Option<..>>`** but
>   no code ever produced `None` (per-column probing can't fail a table
>   wholesale). Simplified to `Vec<Vec<..>>`; removed the `.and_then(Option::
>   as_ref)` combinator chains and the never-exercised M02/M03-suppression arm.
> - **idiom (asked-about): `usize::MAX` sentinel → `Option<usize>`** in
>   instantiate_table; typedefs now **borrowed** into the fixpoint (no
>   per-table clone); fixpoint carries each error **with** its index (dropped
>   the parallel `Vec<Option<String>>` + silent `unwrap_or_default`).
> - **idiom: `&dyn Fn` filter → `Option<&str>` + `table_selected` helper**;
>   two hand-built 10-field `Problem` literals (M04/M07) → `Problem::unlocated`
>   constructor.
> - **decision recorded + tested: `examples`/`values` are NOT type-checked in
>   rich mode** (only `range` bounds are). They are illustrative/categorical
>   documentation; M01's exact type round-trip already pins type correctness.
>   Pinned by `rich_does_not_type_check_examples_or_values`; documented in
>   site/validation.md.
> - **kept on the trait (declined move to a core fn): `classify`.** duckdb owns
>   canonical spellings, so the classifier lives with the backend (documented);
>   `fixture_classify` in core tests is not a drift guard (that is
>   `classify.rs`, pinned against live DESCRIBE) — core tests exercise
>   diff-logic-given-a-classification, noted in the fixture's doc.
> - **declined (house rules): consolidating the range logic** duplicated
>   between `rich.rs` and `validate_spec.rs` — two type vocabularies, short and
>   stable; added reciprocal `keep in step` cross-reference comments instead.
> - **added tests** for the review's gaps: M08+M05 interplay, absolute
>   `source.duckdb.file`, multiple M07s, empty database → M06, case-insensitive
>   match, plus the two security tests above. stale shell-out module doc in
>   `dbdict-duckdb/src/lib.rs` corrected (marked the coarse reader transitional,
>   deleted in phase 4); site/validation.md "scratch tables" reworded to
>   per-column and the case-insensitive rule noted.

### phase 4: CLI + docs + polish — DONE 2026-07-06T12:46:23+12:00

> **decisions (2026-07-06, asked at phase start):** include the `resolve`
> command (yes); rich *data* level (D01 via duckdb) deferred to a future
> session — the `RichFormatUnsupported` pre-flight stays; spec.md is ONE doc,
> rich-first, with legacy 0.1.0 in its own clearly-marked section.

- [x] CLI: `types duckdb <db>` (native, via `read_schema`) — prints every
      relation (tables and views) with canonical column types, `─` rule under
      each header matching the parquet printer. `validate-meta` wiring needed
      nothing (done in phase 3).
- [x] CLI: `resolve [dict]` — prints `name  declared-expr  → canonical` for
      every typedef: globals first, then per-table groups; duckdb's error
      inline and exit 1 when an alias is unknown/cyclic/malformed; legacy
      dict → `(no typedefs)`, exit 0. Backend: `dbdict_duckdb::
      expand_typedefs` + `TypedefExpansion`, sharing the fixpoint/probe
      machinery via `effective_typedefs` + `probe_type` helpers factored out
      of `instantiate_table` (which got simpler); core's `load_and_lower`
      made public — the model entry point for the CLI and future generators.
- [x] deleted the dead shell-out reader: `describe`/`column_types`/
      `parse_describe`/`ColumnTypeInfo`/`DuckdbError`/`DictType`/
      `dict_type_for`, `src/error.rs`, `src/types.rs`, `tests/describe.rs`;
      `quote_ident` moved into native.rs; serde_json dep dropped from
      dbdict-duckdb. No `duckdb` CLI needed by anything anymore.
- [x] docs: `site/spec.md` rewritten rich-first ($version discriminator,
      typedef, dict-level `source.duckdb`, duckdb-native types with verified
      duckdb-docs links, `label`; legacy 0.1.0 section keeps coarse types /
      measures / per-table parquet). `README.md` rewritten for dbdict (fork
      note, rich example, command listing kept in step with the CLI
      snapshot, 4-crate layout). Consistency fixes found on the way:
      `data-dict` → `dbdict` command names in validation.md, stale "rework
      pending" comment in schema-0.2.yaml, stale shell-out note in CLAUDE.md.
- **verify:** PASSED 2026-07-06 — `cargo build --workspace --release` green;
      `cargo test --workspace` 205 passed / 0 failed; clippy + rustfmt clean;
      release binary ran `validate-meta` (clean rich round-trip + correct
      M07), `types duckdb`, and `resolve` with `duckdb` **stripped from
      PATH** (self-contained confirmed).

> **3-agent review (2026-07-06, correctness / idiom / tests+docs) — all
> findings actioned same day, each fix re-verified against the reviewer's own
> repro; two items deliberately deferred (below).**
> - **BUG (correctness, verified end-to-end): `resolve` contradicted
>   `validate-meta`** when a table shadowed a *dependency* of an unshadowed
>   global (`a: intish` global; table redefines `intish`) — validation
>   instantiates that table with the reshaped `a`, but resolve only printed
>   the global expansion. FIXED: the per-table pass now emits an entry for
>   any global whose expansion *differs* in the table's context (outcome
>   comparison, no dependency-graph parsing). Pinned by
>   `shadowed_dependency_reshapes_a_global_in_table_context`.
> - **dead field (all three reviewers, independently): `TypedefExpansion.
>   expr` was never printed** though its doc + test claimed the CLI shows it.
>   FIXED by printing it — `money  DECIMAL(12, 2)  → DECIMAL(12,2)` — which
>   is also just better output.
> - cosmetic leading blank line (scoped-only dicts) fixed; `probe_type` doc
>   corrected (single-row check is a tripwire, not a boundary — statement
>   smuggling only lets a dict lie about its own expected side; safety basis
>   is open_scratch); `expansion_for` → `expansion_result` with the
>   fixpoint position-coupling documented and the `index`→`position` naming
>   aligned with instantiate_table; keep-in-step notes on the fixture
>   builders duplicated between tests/instantiate.rs and tests/expand.rs.
> - **docs accuracy (docs are high-severity here):** legacy-format anchor
>   Quarto-proofed with an explicit `{#…}` id (pandoc keeps dots in auto
>   ids); the "time_zone implies zoneless TIMESTAMP" NB reworded (rich S14
>   accepts both timestamp categories); inherited-but-false "(may include
>   globs)" deleted; "never locks it" → "for writing"; case-insensitivity
>   qualified as ASCII folding (matches names_eq); "descriptive keys"
>   terminology collision in the legacy intro resolved; last `data-dict.yaml`
>   mention in validation.md reworded.
> - **tests added** for the review's gaps: 4 CLI e2e (resolve output
>   snapshot, broken-typedef → exit 1 with inline error, legacy → "(no
>   typedefs)" exit 0, types duckdb snapshot) + stalled-*scoped*-typedef
>   position guard in expand.rs. Workspace 199 → 205 tests.
> - **deferred (user decisions, not defects):** `$learn_more` recommended URL
>   and `site/CNAME` still point at `data-dict.tidyverse.org` — fork
>   branding/domain call; and (pre-existing, phase-3 territory) two dict
>   tables differing only in case pass S10 but would both match one db
>   relation case-insensitively at the meta level.

## open items / risks
- **Spike outcome** (C vs A vs B) — decided in phase 1, recorded there.
- **S-check ripple** — RESOLVED in phase 3: reworked as compatibility checks
  (see the phase-3 decision block); nothing required, misfit combinations
  rejected. S01–S06 unchanged (type-agnostic).
- **Two duckdb sessions** — RESOLVED in phase 3: the real db is opened
  read-only; scratch schemas live in per-table in-memory connections (which
  is also what makes typedef shadowing possible). No cross-session
  collisions by construction.
- **Build cost:** first bundled build is multi-minute + large binary; acceptable
  (self-contained). Subsequent builds cached.
- Retire the phase-1 feature-gate scaffolding — RESOLVED before phase 4 (no
  `[features]`/`cfg(feature)` anywhere in the workspace).
- Phase 4 addition (from phase 3) — DONE in phase 4: `dict_type_for` + the
  shell-out `describe`/`column_types` deleted with the dead reader.
- **Deferred beyond this session:** rich *data* level (D01 via duckdb) to
  replace the `RichFormatUnsupported` pre-flight in `validate-data` (decided
  2026-07-06); fork branding — `$learn_more` URL + `site/CNAME` still point
  at `data-dict.tidyverse.org`; case-insensitive table-name collision (two
  dict tables differing only in case pass S10 but both match one db relation
  at the meta level — pre-existing).
