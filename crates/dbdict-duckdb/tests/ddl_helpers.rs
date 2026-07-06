//! Tests for the helpers the DDL generator (crates/dbdict-ddl) builds on:
//! `typedef_creation_order` (which order a flat script must CREATE TYPE in,
//! discovered by executing against a scratch database) and
//! `execute_and_describe` (prove a script executes, and read back what it
//! built). `quote_ident` is exercised here too, as the public export the
//! generator uses to spell identifiers.
//!
//! Typedef fixtures are built by hand with placeholder spans, like
//! tests/instantiate.rs.

use dbdict::model::{Spanned, Typedef};
use dbdict_duckdb::{execute_and_describe, quote_ident, typedef_creation_order};
use quarto_source_map::SourceInfo;

fn typedef(name: &str, expr: &str) -> Typedef {
    Typedef {
        name: Spanned::new(name.to_string(), SourceInfo::for_test()),
        expr: Spanned::new(expr.to_string(), SourceInfo::for_test()),
    }
}

#[test]
fn creation_order_puts_dependencies_first() {
    // `big` compounds on `money` but is declared first: the returned order
    // must put `money` (index 1) before `big` (index 0) or a flat script
    // would fail
    let tds = [
        typedef("big", "money[]"),
        typedef("money", "DECIMAL(18, 4)"),
        typedef("address", "STRUCT(city VARCHAR, postcode INTEGER)"),
    ];
    let refs: Vec<&Typedef> = tds.iter().collect();

    let order = typedef_creation_order(&refs).expect("all typedefs resolve");

    // every typedef appears exactly once
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(sorted, vec![0, 1, 2], "got order {order:?}");
    let pos = |i: usize| order.iter().position(|&x| x == i).unwrap();
    assert!(
        pos(1) < pos(0),
        "money (1) must be created before big (0), got {order:?}"
    );
}

#[test]
fn creation_order_reports_the_stalled_group() {
    let tds = [
        typedef("ok_type", "VARCHAR"),
        typedef("cyc_a", "cyc_b"),
        typedef("cyc_b", "cyc_a"),
        typedef("dangling", "NO_SUCH_TYPE"),
    ];
    let refs: Vec<&Typedef> = tds.iter().collect();

    let stalled = typedef_creation_order(&refs).expect_err("the cycle must stall");

    let mut failed: Vec<usize> = stalled
        .iter()
        .map(|(index, error)| {
            assert!(
                !error.is_empty(),
                "a stalled typedef carries duckdb's error"
            );
            *index
        })
        .collect();
    failed.sort();
    assert_eq!(failed, vec![1, 2, 3], "got {stalled:?}");
}

#[test]
fn creation_order_of_nothing_is_empty() {
    let order = typedef_creation_order(&[]).expect("nothing to create");
    assert!(order.is_empty());
}

#[test]
fn execute_and_describe_reads_back_what_the_script_built() {
    let script = "CREATE TYPE money AS DECIMAL(12, 2);\n\
                  CREATE TABLE trades (qty BIGINT, price money);\n\
                  CREATE TABLE accounts (id INTEGER);";

    let schemas = execute_and_describe(script).expect("script executes");

    // alphabetical by relation name, canonical types (the typedef expanded)
    assert_eq!(schemas.len(), 2, "got {schemas:?}");
    assert_eq!(schemas[0].name, "accounts");
    assert_eq!(
        schemas[0].columns,
        vec![("id".to_string(), "INTEGER".to_string())]
    );
    assert_eq!(schemas[1].name, "trades");
    assert_eq!(
        schemas[1].columns,
        vec![
            ("qty".to_string(), "BIGINT".to_string()),
            ("price".to_string(), "DECIMAL(12,2)".to_string()),
        ]
    );
}

#[test]
fn execute_and_describe_fails_on_a_broken_script() {
    let error = execute_and_describe("CREATE TABLE t (x NO_SUCH_TYPE);")
        .expect_err("a broken script must fail");
    assert!(!error.is_empty());
}

#[test]
fn execute_and_describe_cannot_reach_the_filesystem() {
    // scripts run in the same sandboxed scratch database as instantiation:
    // external access is disabled, so ATTACH must fail rather than write
    let mut marker = std::env::temp_dir();
    marker.push(format!("dbdict-ddl-attack-marker-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let script = format!("ATTACH '{}' AS pwn;", marker.display());

    let result = execute_and_describe(&script);

    assert!(!marker.exists(), "a script must not create a file on disk");
    assert!(result.is_err(), "the ATTACH must be rejected");
}

#[test]
fn quote_ident_is_exported_for_the_generator() {
    assert_eq!(quote_ident("food"), "\"food\"");
    assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
}
