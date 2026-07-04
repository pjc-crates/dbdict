# implementation: DuckDB source support (meta-first)

Branch: `duckdb-source`. Verified against `duckdb` 1.5.4 on PATH.

> Feature gate (**default-off**): duckdb support lives behind a `duckdb` cargo
> feature, off by default.
> - **Ungated (always compiled):** the `duckdb` key in `schema.yaml`, model
>   `SourceKind::Duckdb`, and lowering — so `validate-spec` accepts and documents
>   a duckdb source in *any* build, including the lean parquet-only default.
> - **Gated behind `duckdb`:** the `data-dict-duckdb` dependency, the seam's
>   duckdb branch, and the `types duckdb` subcommand.
> - **Failure modes:** feature off + duckdb source at meta/data → clear
>   "duckdb source not supported in this build"; feature on + duckdb missing →
>   runtime "duckdb CLI not found on PATH" preflight.
> - **Obligation:** CI/tests MUST cover both configs (default and
>   `--features duckdb`) or the `#[cfg]`'d code silently rots.

## phases

### phase 1: `data-dict-duckdb` reader crate (standalone) — DONE 2026-07-04T19:37:09+12:00

Self-contained shell-out reader, testable in isolation with zero core changes.

- [x] add crate `crates/data-dict-duckdb/` + register in `Cargo.toml` workspace
      `members` and `[workspace.dependencies]`
- [x] `Cargo.toml`: deps = `serde_json` (workspace). No `duckdb` crate, no arrow.
- [x] `src/error.rs`: `DuckdbError` enum — `NotFound` (duckdb not on PATH),
      `Cli { status, stderr }`, `Parse(String)`. `Display` impls for M05 messages.
- [x] `src/types.rs`: `dict_type_for(column_type: &str) -> DictType` — normalise
      (uppercase, strip `(...)`/`[]`), then map per the goal's table
      (number/string/boolean/date/datetime/enum; TIME/BLOB/UUID→string;
      LIST/STRUCT/MAP→`Unsupported`). Prefix-match TIMESTAMP* and ENUM*.
- [x] `src/lib.rs`: `ColumnTypeInfo { name, dict_type, duckdb_type }`;
      `describe(file, table)` running `duckdb -readonly -json <file> -c
      'DESCRIBE "<table>";'`, parse the JSON array, read `column_name` +
      `column_type`. `column_types()` projects to `Vec<(name, dict_type_string)>`.
- **verify (automated):** `cargo test -p data-dict-duckdb` — **17 tests pass**
      (14 lib incl. every type-table row + JSON parse + identifier quoting; 3
      integration building a temp `.duckdb`). clippy clean; `cargo build
      --workspace` green.
- **verify (manual):** covered by integration test `describe_reads_columns_and_
      maps_types` (real temp db → asserted (name, dict_type, duckdb_type)).
- also: TDD'd — cycle A (pure `parse_describe`/`quote_ident`, unit-tested with
      the captured v1.5.4 JSON) then cycle B (shell-out `describe`, integration
      test). integration tests skip with a notice when duckdb is absent so CI
      without duckdb doesn't hard-fail.

### phase 2: schema + model + lowering + core seam → `validate-meta` end-to-end

Accept `duckdb` at spec level, enforce one source method, and wire the phase-1
reader into the validation seam.

- [ ] `schema.yaml`: add `duckdb` to `source.properties` as
      `anyOf: [string, {closed object, required:[file], props: file, table}]`.
      Enforce one method: add `maxProperties: 1` to the `source` object.
      **Open item:** confirm the quarto-yaml-validation dialect supports
      `maxProperties`; if not, drop it and enforce via a new S-check (S18
      "multiple source methods") in `validate_spec.rs` + `ProblemKind` in
      `problem.rs`.
- [ ] `crates/data-dict/src/model.rs`: replace `Source { parquet }` with a kind
      enum — `Source { span, kind: SourceKind }`, `SourceKind::Parquet(Spanned<String>)`
      | `SourceKind::Duckdb { file: Spanned<String>, table: Option<Spanned<String>> }`.
- [ ] `crates/data-dict/src/lower.rs`: lower both forms (string vs object) into
      `SourceKind`; carry spans for diagnostics.
- [ ] `crates/data-dict/Cargo.toml`: `data-dict-duckdb = { workspace = true,
      optional = true }`; `[features] duckdb = ["dep:data-dict-duckdb"]`; no
      default features (duckdb off by default). Confirm `dep:` syntax vs cargo docs.
- [ ] `crates/data-dict/src/lib.rs`: rename `read_parquet` → `read_source`;
      dispatch on `SourceKind`. Parquet branch unchanged. Duckdb branch:
      - `#[cfg(feature = "duckdb")]`: resolve `file` relative to dict dir,
        `table` = specified or the dict table's `name`; call
        `data_dict_duckdb::column_types`; map `DuckdbError` → M05
        `UnreadableSource` (incl. a clear `NotFound`/on-PATH message).
      - `#[cfg(not(feature = "duckdb"))]`: emit a clear problem — "duckdb source
        not supported in this build" (distinct message; not "unreadable").
- [ ] confirm `validate_meta.rs` M01 type-compat treats duckdb-reported dict
      types identically to parquet's (same coarse strings) — adjust if not.
- **verify (automated, BOTH configs):**
      - default (feature off): `cargo test --workspace` — compiles, spec tests
        for duckdb lowering/one-source pass, and `validate-meta` on a duckdb
        source yields "not supported in this build".
      - feature on: `cargo test -p data-dict --features duckdb` — the seam's
        duckdb branch compiles and `validate-meta` reaches the reader.
      - new unit tests: lowering a string duckdb source, an object form, and
        dual-source rejection.
- **verify (manual):** dict with `duckdb:` → `validate-spec` ok (either config);
      dict with both `parquet:`+`duckdb:` → error; with `--features duckdb`,
      `validate-meta` against `verify.duckdb` → `ok`; wrong type → M01; dropped
      documented column → M02; undocumented db column → M03. Without the feature,
      the same duckdb dict at meta → "not supported in this build".

### phase 3: CLI `types duckdb` + end-to-end tests

- [ ] `crates/data-dict-cli/Cargo.toml`: `[features] duckdb =
      ["data-dict/duckdb", "dep:data-dict-duckdb"]`, default off; add
      `data-dict-duckdb` as an optional dep.
- [ ] `crates/data-dict-cli/src/main.rs`: add `#[cfg(feature = "duckdb")]`
      `TypesCommand::Duckdb { file, table }` → call `data_dict_duckdb::describe`,
      print a table (`#`, `column`, `dict type`, `duckdb type`) via a small
      printer alongside `print_types_table`. When the feature is off, the variant
      simply doesn't exist (clap reports "unknown subcommand").
- [ ] the no-arg subcommand listing (`subcommands_listing`) then shows
      `types duckdb` **only** in feature-on builds (automatic via the tree walk —
      verify both configs).
- [ ] `crates/data-dict-cli/tests/cli.rs`: `#[cfg(feature = "duckdb")]` e2e insta
      tests for `types duckdb` and `validate-meta` on a duckdb source, building a
      temp db in-test. Gate on `duckdb` presence (skip w/ message if not on PATH)
      so tests don't hard-fail without duckdb installed.
- **verify (automated, BOTH configs):** default `cargo test --workspace`
      (no `types duckdb`, no duckdb tests compiled); `cargo test -p data-dict-cli
      --features duckdb` (cli/e2e tests run).
- **verify (manual):** with `--features duckdb`:
      `data-dict types duckdb scratchpad/verify.duckdb t` prints the type table
      and `data-dict` (no args) lists `types duckdb`. Default build:
      `types duckdb` is an unknown subcommand.

### phase 4: docs + polish

- [ ] `site/spec.md` (Source section): document the `duckdb` source (string /
      `{file, table}`), the one-source rule, and that only names+types are
      validated (meta-level).
- [ ] `site/validation.md`: note duckdb sources under meta validation; mention
      data-level is Parquet-only for now.
- [ ] `README.md`: add duckdb to the CLI capability list + `types duckdb`, and
      document that it is **opt-in at build time**: `cargo install --features
      duckdb` (default build is parquet-only), requires `duckdb` on PATH at runtime.
- [ ] `schema.yaml` header comment: mention duckdb + the build feature + runtime
      dependency.
- [ ] confirm green in BOTH configs: default `cargo build --workspace --release`
      / `cargo test --workspace`, and `cargo build -p data-dict-cli --release
      --features duckdb` / `cargo test -p data-dict-cli --features duckdb`;
      `git status` clean except intended files.
- **verify (automated):** default `cargo test --workspace` + `cargo test
      -p data-dict-cli --features duckdb`.
- **verify (manual):** re-read rendered docs for accuracy; every prose claim
      about duckdb behaviour traces to verified v1.5.4 output.

## open items (resolve during impl, not blockers)

- `maxProperties` support in the schema dialect (phase 2) — else S-check.
- exact handling of `Unsupported` column types at M01 (surface as a distinct
  problem vs. a type mismatch) — decide in phase 2.
- CI duckdb availability for e2e tests (phase 3) — gate/skip when absent.
- two-config CI: both default and `--features duckdb` must be built/tested or the
  `#[cfg]`'d code rots (phases 2–3). Add a `--features duckdb` job to
  `.github/workflows/` in phase 4 if a CI workflow exists.
- exact cargo invocation for enabling `duckdb` across the workspace (per-package
  `-p … --features duckdb` vs a workspace-wide flag) — settle in phase 2.
