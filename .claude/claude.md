# dbdict

`dbdict` is a DuckDB-native data dictionary tool. `dbdict.yaml` describes a collection of related tables — their columns, exact DuckDB types (via a `typedef:` alias layer), constraints, relationships, and domain vocabulary — and the `dbdict` CLI validates a dictionary against the spec, against a database's schema, and against the data's actual values. It can also generate executable DuckDB DDL.

## Lineage

`dbdict` began as a fork of the original `data-dict.yaml` project and has deliberately moved away from it: we commit to DuckDB and exact type fidelity rather than backend-neutral portability, and we do not track the original project's development. The legacy `data-dict.yaml` (v0.1.0, coarse semantic types, parquet sources) format is still parsed and validated so existing dictionaries keep working; the rich `dbdict.yaml` (v0.2.0) format is the direction. Upstream attribution lives in README.md, LICENSE, the repo-root CLAUDE.md, and the site's lineage note (site/index.md) — don't reference the original project anywhere else.

The repo contains:

- `README.md`: project overview, CLI install/build instructions.
- `site/`: the [Quarto](https://quarto.org) website source (currently unpublished — publishing is a future decision). Holds the spec (`spec.md`), the validation rules (`validation.md`), design docs (`semantic-models.md`), and legacy-format example dictionaries downloaded from other repos (see `download-examples.R` at the repo root).
- `crates/`: Rust workspace (see crate architecture below)
- `schema.yaml` / `schema-0.2.yaml`: JSON Schemas for structural validation of the legacy and rich formats respectively (embedded into the core crate at compile time; `validate_spec.rs` picks one by the document's `$version`)

## Code principles

* Comment style and general Rust conventions live in the repo-root `CLAUDE.md` — follow it (training-wheels comments explaining the why and any idiom in play; no fancy/clever Rust; the maintainer is learning Rust).
* User facing code should be accompanied by a test.

## Spec and implementation must stay in sync

The spec (`site/spec.md` + validation details in `site/validation.md`) and the implementation (the crates + the schema files) are two views of the same thing and must never drift apart.

- **New features start in the spec, and REQUIRE human sign-off.** This is the single most important rule in this file. Any new feature is a two-phase process with a hard stop between the phases:
    1. **Write the spec.** Draft and iterate the change in `site/spec.md` *only*. Do not touch the schema files, the crates, the tests, or any other file in this phase.
    2. **Stop and get an explicit "yes" from a human on the spec text.** Asking clarifying questions is not sign-off. Presenting a plan is not sign-off. You must show the human the actual spec wording and wait for them to explicitly approve *that wording* before writing a single line of implementation. If you are unsure whether you have approval, you do not have approval — ask again.

  Only after that explicit yes do you implement (schema files, crates, tests). Starting implementation before the human has signed off on the spec is a process violation, even if the feature itself is fine.
- **Implementation refinements flow back to the spec.** If you discover during implementation that the spec is wrong, incomplete, or ambiguous, update `site/spec.md` to match what you actually built.
- **Touch one, check the other.** Whenever you change the spec, double-check the implementation still matches; whenever you change the implementation, update the spec. A change to either is incomplete until both agree.

## Commands

```bash
# Build
cargo build --workspace
cargo build --workspace --all-targets   # includes tests, examples, benches

# Test
cargo test --workspace
cargo test -p dbdict                    # single crate
cargo test -p dbdict spec               # tests matching "spec" in the core crate

# Format and lint (run before committing Rust changes)
cargo fmt --all
cargo clippy --workspace --all-targets

# Validate a file
cargo run -p dbdict-cli -- validate-spec site/examples/otters.yaml
```

To review/accept insta snapshots: `cargo insta review` (if cargo-insta is installed; otherwise review the `.snap.new` diff by hand and `mv` it over the `.snap`).

## Crate architecture

Rust workspace of library crates + a thin CLI. The core is a pure library exposing the parsed/resolved dictionary model; the CLI and generators consume that model — they never touch YAML or each other.

- `crates/dbdict/` — core: model, source-mapped YAML parse, typedef resolution, validation engine, diagnostics. All logic lives here.
- `crates/dbdict-duckdb/` — DuckDB backend (native bundled `duckdb` crate).
- `crates/dbdict-parquet/` — parquet backend (legacy path only).
- `crates/dbdict-ddl/` — DDL generator.
- `crates/dbdict-cli/` — thin CLI wrapper (binary: `dbdict`). Keep it thin.

### Validation levels

The three levels and every check code (`S##` / `M##` / `D##`) are defined in `site/validation.md` — the single source of truth. Don't re-document the checks here or in code comments; point to that file. Each level implies the ones before it.

Implementation in the core crate: `validate_spec.rs` covers the spec level for both formats (it picks the embedded schema by `$version`). For the metadata and data levels the two formats split: the legacy path lives in `validate_meta.rs` / `validate_data.rs`, the rich path in `rich.rs` behind the `DuckdbBackend` seam (implemented by `dbdict-duckdb`). `Level`, the `select_tables` helper, and the legacy `compare_parquet` driver live in `lib.rs`. Entry points are re-exported at the crate root.

Test fixtures for the spec rules are in `crates/dbdict/tests/fixtures/spec/`. Each fixture has a `# expected: ...` header documenting the intended outcome. Integration tests mirror the levels — `tests/validate_spec.rs` / `validate_meta.rs` / `validate_data.rs` for the legacy path, `tests/rich.rs` / `rich_meta.rs` / `rich_data.rs` for the rich path.

### Problem reporting

Two principles guide how problems are surfaced:

- **Full context.** A problem should carry enough context that the user can see at a glance where it comes from — point at the offending span and fade in its enclosing nodes (e.g. the table and column a bad value sits in), so the location is unambiguous without re-reading the file.
- **Report as many problems as possible at once.** Prefer collecting all the problems in a pass over bailing on the first, so the user fixes them together rather than rerunning repeatedly. Not always possible (a problem can block the checks that would follow it), but worth striving for.

### Diagnostic wording

A diagnostic is split across two parts: `expected` is a general statement of the problem, and `message` reports what was found at the offending location. `expected` leads the rendering (the title line beside the code for span-located spec problems; the headline line for the plain-rendered metadata/data problems) and `message` follows it. Prefer this split whenever a general rule can be stated, at every level (`S`/`M`/`D`).

- `expected` is one concise but informative statement, in sentence case, ending with a full stop. State what *must* hold when the cause is clear (e.g. an incorrect type or size: "A range's minimum must be less than or equal to its maximum."); use *can't* when you can't state what was expected.
- `message` (the "found" detail) is a lowercase fragment with no full stop — it names the concrete value or location ("minimum `100` is greater than maximum `10`").
- Diagnostic hints always start with a capital letter.

If a change causes the `site/examples/` dictionaries to fail validation, don't fix them — they are downloaded legacy examples (see `download-examples.R`); report the failure instead.

## Data format

- Keys in dictionary files use snake_case (e.g. `primary_key`, `foreign_key`, `$learn_more`).

## Prose

- Use sentence case for headings.
