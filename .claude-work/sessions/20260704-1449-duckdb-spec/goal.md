# DuckDB rich-type data dictionary — typedef alias layer

> Redefinition (2026-07-04). Supersedes the original coarse/portable/upstream
> goal. Phase 1 (the shell-out duckdb reader) is a completed, reusable step and
> survives the pivot: it already returns full native type spellings.

## problem

We are changing direction: **not** a portable, backend-agnostic dictionary with
coarse semantic types, **but** a rich, DuckDB-native schema tool. Columns are
typed in DuckDB's own (rich) type domain — `STRUCT`, `ENUM`, `LIST`, arrays,
`DECIMAL(p,s)`, etc. — and a readable **alias layer** (`typedef:`) lets you name
and reuse those native types. Validation checks the actual DuckDB database
against the dictionary with **full type fidelity**, not coarse categories.

Upstream/portability is explicitly dropped. DuckDB-first.

**Dual-format (decided 2026-07-04):** the legacy `data-dict.yaml` path (coarse
types + parquet) is **preserved** so existing/upstream files keep validating;
the new rich/duckdb path (`dbdict.yaml`) is **added alongside**, not a
replacement. The tool renames to `dbdict` and supports both file conventions.

## success criteria

- A `typedef:` section (global and/or table-scoped) maps `name → native type
  expression` in DuckDB SQL syntax (copy-pasteable). Aliases **compound**
  (an alias may appear inside another's expression).
- A column's `type:` is either an alias name or a native DuckDB type expression.
- `validate-meta` checks each column's declared type (alias-expanded, canonical)
  against the actual DuckDB column type with **full fidelity** (struct fields,
  array sizes, decimal precision, enum values), reported with source-span
  diagnostics.
- Clear errors for: type mismatch, unknown alias, malformed typedef, cyclic
  typedef, alias/native ambiguity.
- The coarse vocabulary (`number/string/…`) and `types_compatible` are **kept**
  for the legacy `data-dict.yaml` path; the rich path is additive, not a rewrite.
- Existing engine kept: S01–S17 spec checks, YAML source-map parsing,
  `annotate_snippets` diagnostics, M02–M05, CLI. **Stay in Rust; extend, don't
  rewrite.**

## scope

- **in:**
  - `typedef:` parsing — global + table-scoped, table shadows global.
  - alias **resolution** (own the graph: topo-order, cycle-detect, source-span
    errors) — see open decision below.
  - **fidelity type comparison** via DuckDB canonicalization (not a hand-rolled
    DuckDB type-grammar parser).
  - schema/model/lower changes: `type:` becomes alias-or-native; add `typedef:`.
  - `validate-meta` rewrite: alias-expand → canonical native → match actual.
  - rework the phase-1 `data-dict-duckdb` reader onto the native crate; the
    coarse `dict_type_for` retires (rich direction) — DESCRIBE/type-string
    knowledge carries over.
- **out (deferred / not now):**
  - Parquet alias/rich-type support (DuckDB-first; parquet is a different type
    domain — cross-domain mapping is later work).
  - Data-level (`D##`) value checks for duckdb.
  - Upstream contribution/portability (the old shell-out CLI reader is replaced
    by the native crate). Note: legacy coarse/parquet is *kept* for
    `data-dict.yaml`, not dropped.

## constraints

- Use the official native **`duckdb` crate, bundled** (duckdb-rs `~1.10504.x` =
  DuckDB 1.5.4) — in-process, `Connection::open_in_memory()`. Self-contained
  binary; **no runtime `duckdb`-on-PATH dependency**. No cargo feature gate
  (duckdb is mandatory now).
- **Do NOT assume DuckDB can do all the substitution/canonicalization for us.**
  Prove it with an early spike (below); keep a fallback.
- Canonicalization is delegated to DuckDB (`typeof`/`CAST`/`DESCRIBE`), never
  reimplemented — whichever component expands the aliases.
- Keep the existing Rust engine and its diagnostics; this is an extension.
- **Comment style (user is learning Rust):** thorough but concise
  "training-wheels" comments — lowercase, no trailing periods, no fancy/clever
  Rust, explain the *why* + idioms/gotchas. Readability and maintenance cost
  over execution speed.

## how validation actually works (the crux)

The actual DB columns are **native-typed** (DESCRIBE has no aliases); the dict is
**alias-typed**. So aliases must be expanded to native form before comparison.
The check is: `canonical(declared, alias-expanded)` == `canonical(actual)`.

Candidate mechanisms — **decided by the spike, not assumed**:

- **C. Round-trip (leading).** Generate DDL from the dict — `CREATE TYPE` per
  typedef + `CREATE TABLE` per table using alias/native column types — run it
  into a scratch **in-memory** connection (`Connection::open_in_memory()`), then
  `DESCRIBE` the instantiated tables → canonical `(name, type)`. Compare to the
  real db's `DESCRIBE`. We do **zero** substitution: DuckDB expands + canonicalizes;
  both sides are `DESCRIBE` output from the *same* engine, so directly comparable.
  Bonus: the dict-side `DESCRIBE` is already the `Vec<(name, type)>` shape
  `meta_issues` consumes, so M01/M02/M03 become a diff of two `(name, type)`
  lists; and dict coherence is validated for free (bad/cyclic typedef → DuckDB
  errors on instantiation).
- **A. Per-type offload:** `CREATE TYPE` + `typeof(NULL::<declared>)` per column;
  compare to DESCRIBE. Fallback if whole-table round-trip has issues.
- **B. Own the alias graph:** we resolve/expand aliases ourselves (topo-order,
  cycle detect, source-span diagnostics), DuckDB only canonicalizes. Most
  control + generalizes to parquet later; use if the offloads prove unreliable.

None reimplement DuckDB's canonical spelling. Verified proof-of-concept: a
scratch `CREATE TABLE t (home address)` (alias) DESCRIBEs to
`STRUCT(city VARCHAR, postcode INTEGER)` (duckdb 1.5.4).

Caveats for C: name-only columns can't be materialized (skip; not type-checked);
topo-order the typedefs; keep the dict AST to map scratch columns back to YAML
source spans by name for diagnostics.

## key design decisions

1. **Alias layer only** — we never invent types the backend must create; a
   typedef expands to a backend-native type.
2. **DuckDB-first**; parquet aliases deferred.
3. **Dual-format** — legacy `data-dict.yaml` (coarse/parquet) preserved; new
   `dbdict.yaml` (rich/duckdb) added. For the rich path, type authority is the
   backend domain.
4. **Resolution mechanism is OPEN, C leading** — round-trip the dict through a
   scratch in-memory duckdb and compare `DESCRIBE`-to-`DESCRIBE`. Confirmed by
   the phase-1 spike, with A/B as fallbacks.
5. **Canonicalization delegated to DuckDB**, never reimplemented.
6. **Stay in Rust, extend the existing engine** — the alias feature is small
   (DuckDB does the type resolution); the engine + diagnostics are the asset.
7. **Native bundled `duckdb` crate, not shell-out** — the pivot removed every
   reason for shell-out (portability / upstream / non-duckdb users). In-process
   in-memory fits the round-trip; bundling makes the binary self-contained. The
   default-off feature gate is dropped (duckdb is mandatory).

## future direction (drives current architecture)

The dictionary is a **source of truth meant to drive generation**, not only
validate: planned consumers include dummy-data generators and SQL / client-code
generators for Julia, Python, etc. This is *why* full type fidelity matters (you
can't generate accurate DDL/data/code from coarse types), and why the core stays
a **pure library exposing a rich, resolved dictionary model** that the CLI and
each generator (separate crates, wired as CLI subcommands) consume. Generators
take the model, never YAML or the CLI.

**Rename (pending decision):** `data-dict` → `dbdict` — the tool has diverged
fully from tidyverse data-dict; the shared name misleads. Do it as a discrete
step before more code accretes.

## phase-1 spike (prove before building) — DONE 2026-07-04: C confirmed

Prove the round-trip (C) before committing: materialize a dict (typedefs +
tables) into a scratch in-memory duckdb, and confirm its `DESCRIBE` **byte-matches**
an equivalent real table's `DESCRIBE` across the type zoo — nested `STRUCT`,
`LIST`/`MAP`/`UNION`, fixed `T[N]` vs variable `T[]`, `DECIMAL(p,s)`, `ENUM` —
including field-order/case/whitespace. Also confirm malformed/cyclic typedefs
and unknown aliases error usefully on instantiation. If C holds → use it; else
fall back to A, then B. (Shell-out-vs-native is resolved: native bundled crate.)

## references

- phase-1 reader: `crates/data-dict-duckdb` (returns native `duckdb_type`).
- verified: `CREATE TYPE … AS INTEGER|STRUCT(...)|…` supports aliases +
  compounding + canonicalization (duckdb 1.5.4); DuckDB
  [`CREATE TYPE` docs](https://duckdb.org/docs/current/sql/statements/create_type.html).
