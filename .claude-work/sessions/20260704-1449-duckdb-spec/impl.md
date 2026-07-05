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

### phase 3: `validate-meta` rewrite (full fidelity, round-trip)
- [ ] seam: build the scratch in-memory db from the dict (`CREATE TYPE` +
      `CREATE TABLE`), `DESCRIBE` → **expected** `(name, canonical_type)`; open
      the real duckdb db → **actual** `(name, canonical_type)`; diff.
- [ ] resolve `source.duckdb.file` **relative to the dict** (absolute used
      as-is) — promised in schema/model comments, implemented nowhere yet
      (review finding). replace the transitional "rich not supported at this
      level" pre-flight in `compare_dataset` with the real wiring.
- [ ] table-set diff (new, enabled by dict-level source): dict table missing
      from the db, and undocumented db table, each reported.
- [ ] **rework S07/S08/S12–S15 for rich mode** (moved from phase 2): classify
      the canonicalized column type (via the scratch-db seam) — enum-like →
      `values`, numeric/temporal → `range`, etc.; `units` on numerics;
      `time_zone` on timestamps. legacy mode behaviour unchanged. core stays
      pure — canonicalization injected via a trait implemented in
      `dbdict-duckdb`.
- [ ] reframe M01/M02/M03 as the `(name, type)` diff: type differs → M01 (exact
      canonical string compare), dict-only → M02, real-only → M03. Map problems
      back to dict column **source spans by name**.
- [ ] **retire `types_compatible`** and the coarse comparison.
- [ ] dict coherence via **fixpoint instantiation**: attempt `CREATE TYPE` for
      every typedef in document order, retry rejects until stall; the stalled
      leftovers (cyclic / unknown type) each report duckdb's error at the
      typedef's source span. bad column types likewise report at their span.
- **verify (end-to-end):** validate a dict against a real duckdb db — clean
      match → ok; struct field type wrong → M01 with the exact `STRUCT(...)`
      diff; dropped documented col → M02; undocumented db col → M03.

### phase 4: CLI + docs + polish
- [ ] CLI: `types duckdb` (native), `validate-meta` wiring; consider an
      `expand`/`resolve` command that prints a typedef's canonical expansion.
- [ ] delete the dead shell-out reader code once native replaces it.
- [ ] docs: `site/spec.md` (`typedef:` + rich types), `README.md`; remove the
      retired coarse-type prose.
- **verify:** `cargo build --workspace --release` + `cargo test --workspace`
      green; confirm the binary runs with **no `duckdb` on PATH** (self-contained).

## open items / risks
- **Spike outcome** (C vs A vs B) — decided in phase 1, recorded there.
- **S-check ripple (important):** the free-form `type:` retires the coarse enum
  that S07/S08/S12–S15 and the column keys `values`/`range`/`examples`/`units`/
  `time_zone` are built on. Decide their fate in phase 2: which survive as
  descriptive metadata, which retire, which rework. S01–S06 (FK/join/cardinality)
  are type-agnostic and survive.
- **Two duckdb sessions:** real db opened (read-only) for actual types; a
  separate in-memory session builds the dict scratch schema. Confirm clean
  separation (no type-name collisions across sessions).
- **Build cost:** first bundled build is multi-minute + large binary; acceptable
  (self-contained). Subsequent builds cached.
- Retire the phase-1 feature-gate scaffolding entirely (duckdb mandatory).
