//! Round-trip tests for the DDL generator: the generated script must execute
//! in a fresh DuckDB and build exactly the schema the dictionary's own
//! instantiation describes (the validate-meta trick, pointed at our output).
//!
//! `DataDict` fixtures are built by hand with placeholder spans, like
//! crates/dbdict-duckdb/tests/instantiate.rs — lowering is core's business.

use dbdict::model::{Column, DataDict, Format, Spanned, Table, Typedef};
use dbdict_ddl::{DdlError, generate};
use dbdict_duckdb::execute_and_describe;
use quarto_source_map::SourceInfo;

fn spanned<T>(value: T) -> Spanned<T> {
    Spanned::new(value, SourceInfo::for_test())
}

fn typedef(name: &str, expr: &str) -> Typedef {
    Typedef {
        name: spanned(name.to_string()),
        expr: spanned(expr.to_string()),
    }
}

fn column(name: &str, col_type: Option<&str>) -> Column {
    Column {
        name: spanned(name.to_string()),
        label: None,
        constraints: Vec::new(),
        col_type: col_type.map(|t| spanned(t.to_string())),
        values: None,
        range: None,
        examples: None,
        units: None,
        time_zone: None,
    }
}

fn table(name: &str, typedefs: Vec<Typedef>, columns: Vec<Column>) -> Table {
    Table {
        name: spanned(name.to_string()),
        label: None,
        typedefs,
        columns,
        source: None,
        description: None,
        details: None,
    }
}

fn dict(typedefs: Vec<Typedef>, tables: Vec<Table>) -> DataDict {
    DataDict {
        format: Format::Rich,
        typedefs,
        source: None,
        extensions: Vec::new(),
        tables,
        relationships: Vec::new(),
    }
}

/// one expected `(column, canonical type)` pair
fn col(name: &str, canonical_type: &str) -> (String, String) {
    (name.to_string(), canonical_type.to_string())
}

#[test]
fn generates_types_in_dependency_order_then_tables() {
    // `big` compounds on `money` but is declared first: the script must
    // CREATE TYPE money before big, or duckdb rejects it
    let d = dict(
        vec![
            typedef("big", "money[]"),
            typedef("money", "DECIMAL(18, 4)"),
        ],
        vec![table(
            "trades",
            Vec::new(),
            vec![column("qty", Some("BIGINT")), column("prices", Some("big"))],
        )],
    );

    let script = generate(&d).expect("a clean dictionary generates");

    let money_at = script
        .find("CREATE TYPE \"money\"")
        .expect("money is created");
    let big_at = script.find("CREATE TYPE \"big\"").expect("big is created");
    assert!(
        money_at < big_at,
        "money must be created before big:\n{script}"
    );
    assert!(script.contains("CREATE TABLE \"trades\""), "got:\n{script}");
    // and the script actually executes, building the expected schema
    let schemas = execute_and_describe(&script).expect("the script executes");
    assert_eq!(schemas.len(), 1);
    assert_eq!(
        schemas[0].columns,
        vec![col("qty", "BIGINT"), col("prices", "DECIMAL(18,4)[]")]
    );
}

#[test]
fn round_trips_structs_enums_decimals_and_arrays() {
    let d = dict(
        vec![
            typedef("money", "DECIMAL(18, 4)"),
            typedef("address", "STRUCT(city VARCHAR, postcode INTEGER)"),
            typedef("side", "ENUM('buy', 'sell')"),
        ],
        vec![table(
            "trades",
            Vec::new(),
            vec![
                column("price", Some("money")),
                column("home", Some("address")),
                column("side", Some("side")),
                column("fills", Some("money[]")),
                column("vec", Some("FLOAT[4]")),
            ],
        )],
    );

    let script = generate(&d).expect("a clean dictionary generates");

    // the round-trip property: executing the script builds exactly the schema
    // the dictionary's own instantiation expects (validate-meta's yardstick)
    let built = execute_and_describe(&script).expect("the script executes");
    let expected = dbdict_duckdb::instantiate(&d);
    assert!(expected.failures.is_empty(), "got {:?}", expected.failures);
    assert_eq!(built.len(), 1);
    assert_eq!(built[0].name, "trades");
    assert_eq!(built[0].columns, expected.tables[0]);
    // spot-check one canonical spelling so the yardstick itself is anchored
    assert!(
        built[0].columns.contains(&col("price", "DECIMAL(18,4)")),
        "got {:?}",
        built[0].columns
    );
}

#[test]
fn multiple_tables_generate_in_document_order() {
    let d = dict(
        Vec::new(),
        vec![
            table("zebra", Vec::new(), vec![column("id", Some("INTEGER"))]),
            table("apple", Vec::new(), vec![column("id", Some("INTEGER"))]),
        ],
    );

    let script = generate(&d).expect("generates");

    let zebra_at = script.find("CREATE TABLE \"zebra\"").expect("zebra");
    let apple_at = script.find("CREATE TABLE \"apple\"").expect("apple");
    assert!(zebra_at < apple_at, "tables keep document order:\n{script}");
}

#[test]
fn scoped_typedefs_are_created_alongside_globals() {
    // a table-scoped typedef with a unique name is fine in a flat script
    let d = dict(
        vec![typedef("money", "DECIMAL(18, 4)")],
        vec![table(
            "trades",
            vec![typedef("qty_t", "UBIGINT")],
            vec![column("price", Some("money")), column("qty", Some("qty_t"))],
        )],
    );

    let script = generate(&d).expect("unique scoped typedefs generate");

    assert!(script.contains("CREATE TYPE \"qty_t\""), "got:\n{script}");
    let built = execute_and_describe(&script).expect("the script executes");
    assert_eq!(
        built[0].columns,
        vec![col("price", "DECIMAL(18,4)"), col("qty", "UBIGINT")]
    );
}

#[test]
fn refuses_a_scoped_typedef_shadowing_a_global() {
    let d = dict(
        vec![typedef("money", "DECIMAL(18, 4)")],
        vec![table(
            "trades",
            vec![typedef("money", "DECIMAL(12, 2)")],
            vec![column("price", Some("money"))],
        )],
    );

    let err = generate(&d).expect_err("shadowing cannot be spelled flat");

    match &err {
        DdlError::Shadowing { collisions } => {
            assert_eq!(collisions.len(), 1, "got {collisions:?}");
            assert_eq!(collisions[0].name, "money");
            assert_eq!(collisions[0].sites, vec![None, Some("trades".to_string())]);
        }
        other => panic!("expected Shadowing, got {other:?}"),
    }
    // the rendered message names the typedef and both sites
    let msg = err.to_string();
    assert!(msg.contains("money"), "got: {msg}");
    assert!(msg.contains("trades"), "got: {msg}");
}

#[test]
fn refuses_scoped_typedefs_colliding_across_tables() {
    let d = dict(
        Vec::new(),
        vec![
            table(
                "trades",
                vec![typedef("local_t", "INTEGER")],
                vec![column("x", Some("local_t"))],
            ),
            table(
                "orders",
                vec![typedef("local_t", "BIGINT")],
                vec![column("y", Some("local_t"))],
            ),
        ],
    );

    let err = generate(&d).expect_err("two tables' scoped typedefs collide");

    match &err {
        DdlError::Shadowing { collisions } => {
            assert_eq!(collisions.len(), 1, "got {collisions:?}");
            assert_eq!(
                collisions[0].sites,
                vec![Some("trades".to_string()), Some("orders".to_string())]
            );
        }
        other => panic!("expected Shadowing, got {other:?}"),
    }
}

#[test]
fn shadowing_detection_folds_ascii_case() {
    // duckdb type names are case-insensitive: `Money` and `money` collide
    let d = dict(
        vec![typedef("Money", "DECIMAL(18, 4)")],
        vec![table(
            "trades",
            vec![typedef("money", "DECIMAL(12, 2)")],
            vec![column("price", Some("money"))],
        )],
    );

    assert!(
        matches!(generate(&d), Err(DdlError::Shadowing { .. })),
        "case-differing names still collide"
    );
}

#[test]
fn refuses_stalled_typedefs() {
    let d = dict(
        vec![
            typedef("ok_type", "VARCHAR"),
            typedef("dangling", "NO_SUCH_TYPE"),
        ],
        vec![table(
            "trades",
            Vec::new(),
            vec![column("name", Some("ok_type"))],
        )],
    );

    let err = generate(&d).expect_err("an uncreatable typedef refuses");

    match &err {
        DdlError::TypedefsStalled { failures } => {
            assert_eq!(failures.len(), 1, "got {failures:?}");
            assert_eq!(failures[0].0, "dangling");
            assert!(!failures[0].1.is_empty(), "duckdb's error is carried");
        }
        other => panic!("expected TypedefsStalled, got {other:?}"),
    }
}

#[test]
fn refuses_when_a_column_type_fails_the_self_check() {
    // a bad column type passes spec validation but cannot execute; generate
    // must refuse rather than hand out a script that fails downstream
    let d = dict(
        Vec::new(),
        vec![table(
            "trades",
            Vec::new(),
            vec![column("status", Some("NO_SUCH_TYPE"))],
        )],
    );

    let err = generate(&d).expect_err("a broken column type refuses");

    match &err {
        DdlError::ScriptFailed { error } => {
            assert!(error.contains("NO_SUCH_TYPE"), "got: {error}");
        }
        other => panic!("expected ScriptFailed, got {other:?}"),
    }
}

#[test]
fn untyped_columns_are_omitted_and_empty_tables_skipped() {
    let d = dict(
        Vec::new(),
        vec![
            table(
                "trades",
                Vec::new(),
                vec![
                    column("qty", Some("BIGINT")),
                    column("note", None), // untyped: makes no type claim
                ],
            ),
            table("memos", Vec::new(), vec![column("body", None)]),
        ],
    );

    let script = generate(&d).expect("generates");

    assert!(
        !script.contains("\"note\""),
        "untyped column omitted:\n{script}"
    );
    assert!(
        !script.contains("CREATE TABLE \"memos\""),
        "a table with no typed columns is skipped:\n{script}"
    );
    // ...but not silently: the script says so
    assert!(script.contains("memos"), "the skip is noted:\n{script}");
    let built = execute_and_describe(&script).expect("the script executes");
    assert_eq!(built.len(), 1);
    assert_eq!(built[0].columns, vec![col("qty", "BIGINT")]);
}

#[test]
fn hostile_names_are_quoted() {
    let d = dict(
        Vec::new(),
        vec![table(
            "we\"ird; DROP TABLE x",
            Vec::new(),
            vec![column("se;lect", Some("INTEGER"))],
        )],
    );

    let script = generate(&d).expect("hostile names are quoted, not parsed");

    let built = execute_and_describe(&script).expect("the script executes");
    assert_eq!(built.len(), 1);
    assert_eq!(built[0].name, "we\"ird; DROP TABLE x");
    assert_eq!(built[0].columns, vec![col("se;lect", "INTEGER")]);
}

#[test]
fn an_empty_dictionary_generates_an_empty_script() {
    let d = dict(Vec::new(), Vec::new());

    let script = generate(&d).expect("nothing to generate is not an error");

    assert!(
        execute_and_describe(&script)
            .expect("an empty script executes")
            .is_empty()
    );
}

#[test]
fn refuses_a_legacy_dictionary() {
    let mut d = dict(
        Vec::new(),
        vec![table(
            "trades",
            Vec::new(),
            vec![column("qty", Some("number"))],
        )],
    );
    d.format = Format::Legacy;

    assert!(
        matches!(generate(&d), Err(DdlError::LegacyUnsupported)),
        "legacy coarse types are not duckdb types"
    );
}
