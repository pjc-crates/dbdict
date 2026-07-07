//! `load_extensions` (the M10 seam) against the real bundled engine, plus
//! the declared-extension wiring in `instantiate`.
//!
//! These tests double as the empirical record of what the bundled duckdb
//! build can LOAD under `enable_external_access(false)` (no filesystem or
//! network, so nothing can be fetched or read from an extension directory —
//! only statically-linked extensions can succeed).

use dbdict::model::{DataDict, Format};
use dbdict_duckdb::load_extensions;

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn json_loads_on_the_bundled_engine() {
    let results = load_extensions(&names(&["json"]));
    assert_eq!(results.len(), 1);
    assert!(
        results[0].is_ok(),
        "json should load on the bundled engine: {:?}",
        results[0]
    );
}

#[test]
fn a_bogus_extension_reports_duckdbs_reason() {
    let results = load_extensions(&names(&["no_such_extension"]));
    assert_eq!(results.len(), 1);
    let reason = results[0].as_ref().unwrap_err();
    assert!(!reason.is_empty());
}

#[test]
fn an_unsafe_name_is_refused_without_reaching_the_engine() {
    // a name outside [a-z0-9_] must never be interpolated into LOAD
    let results = load_extensions(&names(&["json; DROP TABLE x"]));
    assert_eq!(results.len(), 1);
    let reason = results[0].as_ref().unwrap_err();
    assert!(reason.contains("not a valid extension name"));
}

#[test]
fn results_come_back_one_per_name_in_order() {
    let results = load_extensions(&names(&["no_such_extension", "json"]));
    assert_eq!(results.len(), 2);
    assert!(results[0].is_err());
    assert!(results[1].is_ok());
}

#[test]
fn instantiate_canonicalizes_a_json_column_when_json_is_declared() {
    // mirror the fixture helpers in tests/instantiate.rs, but with a
    // declared extension: the JSON column should canonicalize instead of
    // failing its probe
    use dbdict::model::{Column, Spanned, Table};
    use quarto_source_map::SourceInfo;

    let spanned = |s: &str| Spanned::new(s.to_string(), SourceInfo::for_test());
    let dict = DataDict {
        format: Format::Rich,
        typedefs: Vec::new(),
        source: None,
        extensions: vec![spanned("json")],
        tables: vec![Table {
            name: spanned("t"),
            label: None,
            typedefs: Vec::new(),
            columns: vec![Column {
                name: spanned("payload"),
                label: None,
                constraints: Vec::new(),
                col_type: Some(spanned("JSON")),
                values: None,
                range: None,
                examples: None,
                units: None,
                time_zone: None,
            }],
            source: None,
            description: None,
            details: None,
        }],
        relationships: Vec::new(),
    };

    let instantiated = dbdict_duckdb::instantiate(&dict);
    assert!(
        instantiated.failures.is_empty(),
        "JSON column should probe cleanly: {:?}",
        instantiated.failures
    );
    assert_eq!(
        instantiated.tables[0],
        vec![("payload".to_string(), "JSON".to_string())]
    );
}
