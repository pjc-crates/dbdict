# DuckDB source support (meta-first) — design spec

## problem

data-dict validates dictionaries against **Parquet only**. The `source` object
in `schema.yaml` is closed to a single `parquet` key, so a table whose data
lives in a DuckDB database can be *documented* but not *validated* against the
real data — `source: { duckdb: ... }` is rejected at `validate-spec` with
`Q-1-18 Unknown property 'duckdb'`.

Goal: let a table's `source` point at a DuckDB relation and validate the
dictionary's column names and types against that live database.

## success criteria

- `source: { duckdb: … }` passes `validate-spec` (schema accepts it).
- `data-dict validate-meta <dict>` validates every duckdb-sourced table's
  column **names and types** against the database (M01–M05 behaviour), by
  shelling out to the `duckdb` CLI.
- `data-dict types duckdb <file> <table>` prints the column-type table, the
  duckdb analogue of `types parquet`.
- a table declaring **both** `parquet:` and `duckdb:` is rejected (one source
  method per table).
- both build configs green: **default** (parquet-only, `duckdb` feature off —
  the shipped default) and **`--features duckdb`**. The default build carries no
  duckdb code and never spawns a subprocess; a duckdb source at meta/data in the
  default build errors "not supported in this build".
- verified working against `duckdb` v1.5.4 on PATH.

## scope

- **in:**
  - `schema.yaml`: add `duckdb` to the `source` object (string form or
    `{file, table}` object); enforce exactly one source method.
  - `model.rs` / `lower.rs`: represent and lower the duckdb source.
  - new crate `data-dict-duckdb`: shell out to the duckdb CLI, parse the JSON
    schema, map duckdb types → dict types. Mirrors the narrow interface of
    `data-dict-parquet` (meta subset first).
  - core seam (`compare_dataset` / `read_parquet` in `lib.rs`): generalise to
    dispatch parquet vs duckdb by source kind.
  - CLI (`main.rs`): `types duckdb` subcommand; wire duckdb into validate-meta.
- **out (deferred / not now):**
  - value-level (D-level) checks for duckdb sources (`validate-data`) —
    meta-first; null-count/row-sample scans over a set-based source deferred.
  - native/bundled `libduckdb` linking (shell-out only for now).
  - SQL-query sources or schema-qualified names beyond `{file, table}`.
  - other planned source types (R, Python, Posit Connect pins).

## constraints

- **shell-out** to the duckdb CLI — no Rust `duckdb` dependency; `duckdb` must
  be on PATH at runtime. Preflight with a clear "duckdb not found" error.
- **own crate**, mirroring `data-dict-parquet`'s interface, so the link
  strategy stays swappable — a future bundled reader can replace the internals
  behind the same functions without touching the core.
- deps kept minimal: `std::process::Command` + `serde_json` (already in-tree).
  Do **not** pull `arrow` (consistent with the parquet crate dropping it).
- behaviour and type-name strings are **version-sensitive** — pin to and record
  the verified version (**duckdb 1.5.4**).

## key design decisions (confirmed)

1. **Link strategy:** shell-out to the duckdb CLI, isolated in `data-dict-duckdb`.
   Reversible behind the crate interface.
2. **Scope:** meta-first (names + types); data-level deferred.
3. **Source shape:** `duckdb: <file>` (table = the dict table's `name`) **or**
   `duckdb: { file, table }` when the relation name differs.
4. **One source per table:** schema `maxProperties: 1` on `source` (or an
   equivalent S-check).
5. **Edge-case types:** TIME / BLOB / UUID → `string`; nested LIST/STRUCT/MAP →
   report "unsupported type" rather than coerce.
6. **Feature gate (default-off):** duckdb support behind a `duckdb` cargo
   feature, off by default. schema / model / lowering stay ungated (so
   `validate-spec` accepts and documents a duckdb source in any build); the
   `data-dict-duckdb` dep, the seam's duckdb branch, and `types duckdb` are
   gated. Feature-off + duckdb source → "not supported in this build";
   feature-on + duckdb missing → runtime "not found on PATH". CI covers both
   configs.

## verified duckdb specifics (empirical, v1.5.4)

- **Reader call:** `duckdb -readonly -json <file> -c 'DESCRIBE "<table>";'`
  (`-readonly` = never lock/mutate the user's db).
- **Output:** a JSON **array of row objects**, keys
  `column_name, column_type, null, key, default, extra`. Parse the array; read
  `column_name` + `column_type`.
- **Type map** (`column_type` → dict type), matched on a **normalised/prefix**
  form (uppercase, strip `(...)`/`[]`), because duckdb reports resolved,
  parameterised spellings:

  | duckdb `column_type` | dict type |
  |---|---|
  | TINYINT, SMALLINT, INTEGER, BIGINT, HUGEINT, U* , FLOAT, REAL, DOUBLE, `DECIMAL(p,s)`, NUMERIC | `number` |
  | VARCHAR (+ CHAR/BPCHAR/TEXT/STRING) | `string` |
  | BOOLEAN | `boolean` |
  | DATE | `date` |
  | prefix `TIMESTAMP` (incl. `TIMESTAMP WITH TIME ZONE`, `TIMESTAMP_S/_MS/_NS`) | `datetime` |
  | prefix `ENUM` (e.g. `ENUM('happy', 'sad')`) | `enum` |
  | TIME, BLOB, UUID | `string` (edge) |
  | LIST / STRUCT / MAP / arrays | unsupported |

- **Open item for impl:** confirm M01 type-compatibility parity with the parquet
  path (how `validate_meta` compares dict types to reader-reported dict types).

## references

- duckdb docs (all `duckdb.org/docs/current/`): `clients/cli/arguments`,
  `clients/cli/output_formats`, `sql/statements/describe`,
  `sql/data_types/overview`.
- precedent: `crates/data-dict-parquet` (narrow interface, `default-features =
  false` + minimal codecs, no arrow).
- install: `~/.local/bin/duckdb` → `~/.duckdb/cli/1.5.4/duckdb` (official
  install script, local, no sudo).
