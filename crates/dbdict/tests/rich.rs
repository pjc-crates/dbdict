//! Integration tests for the rich (`$version: 0.2.0`) format: schema
//! selection by version, free-form duckdb column types, `typedef:`,
//! top-level `source:`, and `label:`.
//!
//! Kept separate from `validate_spec.rs` (the legacy 0.1.0 tests) so each
//! file exercises exactly one format.

mod common;

use std::path::PathBuf;

use common::{Diagnostic, assert_snapshot, diagnostics};
use dbdict::Severity;
use indoc::indoc;

/// the rich-format boilerplate header, mirroring `common::HEADER` (which is
/// pinned to 0.1.0). two lines, so a `body` written beneath starts at line 3
const RICH_HEADER: &str =
    "$version: \"0.2.0\"\n$learn_more: https://github.com/pjc-wspace/dbdict\n";

/// write `body` beneath the rich header to a temp file and return its path
fn rich(body: &str) -> PathBuf {
    common::write_yaml(&common::temp_dir(), &format!("{RICH_HEADER}{body}"))
}

fn assert_valid_rich(body: &str) {
    let path = rich(body);
    let errors = diagnostics(&path, Severity::Error);
    assert!(
        errors.is_empty(),
        "expected rich document to validate, but:\n{}",
        errors.join("\n"),
    );
}

/// validate `yaml` verbatim (no header), expected to fail, capturing source +
/// rendered errors for snapshotting
fn failing_raw(yaml: &str) -> Diagnostic {
    let path = common::write_yaml(&common::temp_dir(), yaml);
    let errors = diagnostics(&path, Severity::Error);
    assert!(
        !errors.is_empty(),
        "expected document to fail validation, but it passed"
    );
    common::diagnostic(&path, &errors.join("\n"))
}

/// validate `body` beneath the rich header, expected to fail — the rich twin
/// of `validate_spec.rs`'s `failing_dict`
fn failing_rich(body: &str) -> Diagnostic {
    let path = rich(body);
    let errors = diagnostics(&path, Severity::Error);
    assert!(
        !errors.is_empty(),
        "expected rich document to fail validation, but it passed"
    );
    common::diagnostic(&path, &errors.join("\n"))
}

// --- schema selection by $version -----------------------------------------

// a version that is neither 0.1.0 nor 0.2.0 is rejected up front with the
// supported list, rather than falling through to either schema's misleading
// "must be one of" enum error
#[test]
fn unsupported_version_is_rejected_with_supported_list() {
    let d = failing_raw(indoc! {r#"
        $version: "0.3.0"
        tables: []
    "#});
    d.assert_contains(&["0.3.0", "is not a supported spec version"]);
    #[cfg(unix)]
    assert_snapshot!(d);
}

// an unquoted `$version: 0.2` is a yaml float, not a string; it must still
// take the unsupported-version path — falling through to the legacy schema
// would tell a rich-format author the only valid version is "0.1.0"
#[test]
fn numeric_version_gets_unsupported_version_error() {
    let d = failing_raw(indoc! {"
        $version: 0.2
        tables: []
    "});
    d.assert_contains(&["0.2", "is not a supported spec version"]);
}

// the test header quotes `$version` but real documents usually won't; an
// unquoted 0.2.0 cannot parse as a number (two dots), so it arrives as a
// yaml string and selects the rich schema all the same
#[test]
fn unquoted_version_selects_rich_schema() {
    let path = common::write_yaml(
        &common::temp_dir(),
        indoc! {"
            $version: 0.2.0
            $learn_more: https://github.com/pjc-wspace/dbdict
            tables:
              - name: trades
                columns:
                  - name: qty
                    type: BIGINT
        "},
    );
    let errors = diagnostics(&path, Severity::Error);
    assert!(
        errors.is_empty(),
        "expected unquoted 0.2.0 to select the rich schema, but:\n{}",
        errors.join("\n"),
    );
}

// a document declaring `$version: 0.2.0` is validated against the rich
// schema, where a column `type:` is a free-form duckdb type expression, not
// the legacy coarse enum
#[test]
fn rich_version_accepts_free_form_types() {
    assert_valid_rich(indoc! {"
        tables:
          - name: trades
            columns:
              - name: qty
                type: DECIMAL(18, 4)
    "});
}

// --- typedef: --------------------------------------------------------------

// `typedef:` maps alias names to native duckdb type expressions, at the top
// level (visible to every table) and per table (shadowing the global name)
#[test]
fn typedef_global_and_table_scoped() {
    assert_valid_rich(indoc! {"
        typedef:
          money: DECIMAL(18, 4)
          address: STRUCT(city VARCHAR, postcode INTEGER)
        tables:
          - name: trades
            typedef:
              money: DECIMAL(12, 2)
            columns:
              - name: qty
                type: money
    "});
}

// a typedef name given twice in the same scope is rejected — not by an S##
// check of ours but by the schema validator itself, which flags duplicate
// mapping keys structurally. these tests pin that guarantee down so a schema
// dialect change that stopped flagging duplicates would surface here
#[test]
fn duplicate_global_typedef_errors() {
    let d = failing_rich(indoc! {"
        typedef:
          money: DECIMAL(18, 4)
          money: DECIMAL(12, 2)
        tables: []
    "});
    d.assert_contains(&["Duplicate key 'money'"]);
    #[cfg(unix)]
    assert_snapshot!(d);
}

#[test]
fn duplicate_table_typedef_errors() {
    let path = rich(indoc! {"
        tables:
          - name: trades
            typedef:
              money: DECIMAL(18, 4)
              money: DECIMAL(12, 2)
            columns:
              - name: qty
                type: money
    "});
    let errors = diagnostics(&path, Severity::Error);
    assert!(
        errors.iter().any(|e| e.contains("Duplicate key")),
        "expected a duplicate-key error, got:\n{}",
        errors.join("\n")
    );
}

// a non-string typedef key (`123:`, `true:` — yaml parses bare scalars) is an
// error, not a silent drop: the schema's `additionalProperties: string`
// constrains *values* only, so without S18 the alias would vanish from the
// model and its uses would fail later with a baffling "unknown type"
#[test]
fn s18_non_string_typedef_name_errors() {
    let path = rich(indoc! {"
        typedef:
          123: INTEGER
        tables: []
    "});
    let errors = diagnostics(&path, Severity::Error);
    assert!(
        errors.iter().any(|e| e.contains("S18")),
        "expected an S18 non-string typedef-name error, got:\n{}",
        errors.join("\n")
    );
}

// a typedef *value* must be a type-expression string; the schema rejects
// anything else
#[test]
fn typedef_value_must_be_string() {
    let d = failing_rich(indoc! {"
        typedef:
          money: [DECIMAL, 18, 4]
        tables: []
    "});
    d.assert_contains(&["Expected string"]);
}

// an alias the dictionary never defines is NOT a spec-level error: the spec
// level cannot tell an unknown alias from a native duckdb type name, so
// alias existence is duckdb's call, made when the dictionary is instantiated
// in the scratch database (the phase-3 fixpoint). a "helpful" spec-level
// unknown-alias check would contradict that design — this test is the tripwire
#[test]
fn unknown_alias_is_legal_at_spec_level() {
    assert_valid_rich(indoc! {"
        tables:
          - name: books
            columns:
              - name: strategy
                type: nosuchalias
    "});
}

// a table redefining a *global* name is shadowing — by design, not an error
#[test]
fn table_typedef_shadowing_global_is_legal() {
    assert_valid_rich(indoc! {"
        typedef:
          money: DECIMAL(18, 4)
        tables:
          - name: trades
            typedef:
              money: DECIMAL(12, 2)
            columns:
              - name: qty
                type: money
    "});
}

// --- coarse-check gate ------------------------------------------------------

// the legacy coarse-type rules S07/S08/S12–S14 do not run for rich columns:
// every column here would trip one of them under legacy rules (`units` on a
// non-quantity, `time_zone` on a non-`datetime`, `values`/`range` on types
// that demand `examples`), yet the document is valid. deleting the gate in
// `check_spec` must turn this red
#[test]
fn coarse_type_checks_do_not_run_for_rich_columns() {
    assert_valid_rich(indoc! {"
        tables:
          - name: trades
            columns:
              - name: qty
                type: BIGINT
                units: contracts
              - name: executed_at
                type: TIMESTAMP
                time_zone: UTC
              - name: side
                type: VARCHAR
                values: [buy, sell]
              - name: price
                type: DECIMAL(18, 4)
                range: [0, 100000]
    "});
}

// the other half of the gating decision: S15 (time-zone *shape*) is
// type-agnostic and still guards rich documents
#[test]
fn s15_still_fires_for_rich_columns() {
    let d = failing_rich(indoc! {"
        tables:
          - name: trades
            columns:
              - name: executed_at
                type: TIMESTAMP
                time_zone: PST
    "});
    d.assert_contains(&["S15", "is not a valid time zone"]);
}

// --- source: / label: -------------------------------------------------------

// the rich format points at its database once, at the top level: one dict =
// one duckdb database. `label` is an optional display name, on both tables
// and columns
#[test]
fn top_level_source_and_labels() {
    assert_valid_rich(indoc! {"
        source:
          duckdb:
            file: warehouse.duckdb
        tables:
          - name: trades
            label: Trade executions
            columns:
              - name: qty
                label: Quantity
                type: BIGINT
    "});
}

// the source object requires its `file`
#[test]
fn source_duckdb_requires_file() {
    let d = failing_rich(indoc! {"
        source:
          duckdb: {}
        tables: []
    "});
    d.assert_contains(&["Missing required property 'file'"]);
}

// `label` is a display string, nothing else
#[test]
fn label_must_be_string() {
    let d = failing_rich(indoc! {"
        tables:
          - name: trades
            label: [Trade, executions]
            columns:
              - name: qty
                type: BIGINT
    "});
    d.assert_contains(&["Expected string"]);
}

// the per-table `source:` is a legacy-format concept (one parquet file per
// table); a rich table gets its data from the top-level database instead
#[test]
fn rich_rejects_per_table_source() {
    let path = rich(indoc! {"
        tables:
          - name: trades
            source:
              parquet: data.parquet
            columns:
              - name: qty
                type: BIGINT
    "});
    let errors = diagnostics(&path, Severity::Error);
    // pin the *reason*, not just failure — an unrelated error would otherwise
    // keep this green while the schema silently regained per-table sources
    assert!(
        errors
            .iter()
            .any(|e| e.contains("Unknown property 'source'")),
        "expected the rich schema to reject a per-table `source:`, got:\n{}",
        errors.join("\n")
    );
}

// the legacy (0.1.0) format has no `typedef:`; its schema is unchanged and
// keeps rejecting the key
#[test]
fn legacy_version_rejects_typedef() {
    let path = common::write_dict(
        &common::temp_dir(),
        indoc! {"
            typedef:
              money: DECIMAL(18, 4)
            tables: []
        "},
    );
    let errors = diagnostics(&path, Severity::Error);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("Unknown property 'typedef'")),
        "expected the legacy schema to reject `typedef:`, got:\n{}",
        errors.join("\n")
    );
}

// ... and the same for the other two keys only the rich schema knows, so the
// legacy schema accidentally growing any of them fails loudly
#[test]
fn legacy_version_rejects_label() {
    let path = common::write_dict(
        &common::temp_dir(),
        indoc! {"
            tables:
              - name: trades
                label: Trade executions
                columns:
                  - name: qty
        "},
    );
    let errors = diagnostics(&path, Severity::Error);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("Unknown property 'label'")),
        "expected the legacy schema to reject `label:`, got:\n{}",
        errors.join("\n")
    );
}

#[test]
fn legacy_version_rejects_top_level_source() {
    let path = common::write_dict(
        &common::temp_dir(),
        indoc! {"
            source:
              duckdb:
                file: warehouse.duckdb
            tables: []
        "},
    );
    let errors = diagnostics(&path, Severity::Error);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("Unknown property 'source'")),
        "expected the legacy schema to reject a top-level `source:`, got:\n{}",
        errors.join("\n")
    );
}

// --- S10 case folding (rich only) ------------------------------------------

// duckdb identifiers are case-insensitive, so two rich tables whose names
// differ only in case cannot both exist in the dictionary's database — S10
// folds ASCII case for rich documents (matching `names_eq` at the meta level)
#[test]
fn s10_rich_table_names_colliding_by_case() {
    let d = failing_rich(indoc! {"
        tables:
          - name: food
            columns:
              - name: id
                type: BIGINT
          - name: Food
            columns:
              - name: id
                type: BIGINT
    "});
    d.assert_contains(&["S10", "Table names must be unique"]);
    #[cfg(unix)]
    assert_snapshot!(d);
}

// same folding for column names within a table
#[test]
fn s10_rich_column_names_colliding_by_case() {
    let d = failing_rich(indoc! {"
        tables:
          - name: food
            columns:
              - name: id
                type: BIGINT
              - name: ID
                type: VARCHAR
    "});
    d.assert_contains(&["S10", "Column names must be unique"]);
    #[cfg(unix)]
    assert_snapshot!(d);
}
