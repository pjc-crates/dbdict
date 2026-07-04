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

### phase 2: schema + model + `typedef:` parsing
- [ ] `schema.yaml`: add top-level **`typedef:`** (map `name → type-expression
      string`) and table-scoped `typedef:`; change column `type:` from the fixed
      enum to a **free-form string** (alias name or native DuckDB type expr).
- [ ] `model.rs` / `lower.rs`: represent typedefs (global + per-table, table
      shadows global) and column type-as-string, carrying source spans.
- [ ] typedef **resolution**: topo-order + cycle detection. Own it (for
      source-span diagnostics) unless the spike shows DuckDB's errors suffice.
- [ ] **source**: a table points at the real duckdb db + relation
      (`source.duckdb: { file, table }` or string). Resolve relative to the dict.
- **verify:** `cargo test`; lowering tests (global + scoped typedef, shadowing,
      cyclic → error); a dict with `typedef:` parses.

### phase 3: `validate-meta` rewrite (full fidelity, round-trip)
- [ ] seam: build the scratch in-memory db from the dict (`CREATE TYPE` +
      `CREATE TABLE`), `DESCRIBE` → **expected** `(name, canonical_type)`; open
      the real duckdb db → **actual** `(name, canonical_type)`; diff.
- [ ] reframe M01/M02/M03 as the `(name, type)` diff: type differs → M01 (exact
      canonical string compare), dict-only → M02, real-only → M03. Map problems
      back to dict column **source spans by name**.
- [ ] **retire `types_compatible`** and the coarse comparison.
- [ ] dict coherence: a dict whose DDL won't instantiate (bad/cyclic typedef,
      unknown alias) reports a clear spec-level error.
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
