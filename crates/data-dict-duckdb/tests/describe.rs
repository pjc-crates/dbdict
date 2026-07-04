//! Integration tests for the shell-out reader. These build a real temp
//! `.duckdb` via the CLI, so they require `duckdb` on PATH — they skip (with a
//! notice) when it is absent, rather than hard-failing.

use std::path::{Path, PathBuf};
use std::process::Command;

use data_dict_duckdb::{ColumnTypeInfo, DictType, DuckdbError, column_types, describe};

fn duckdb_available() -> bool {
    Command::new("duckdb")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "data-dict-duckdb-test-{}-{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("test.duckdb")
}

fn create(db: &Path, sql: &str) {
    let status = Command::new("duckdb")
        .arg(db)
        .arg("-c")
        .arg(sql)
        .status()
        .expect("run duckdb to build the test db");
    assert!(status.success(), "failed to create test db");
}

#[test]
fn describe_reads_columns_and_maps_types() {
    if !duckdb_available() {
        eprintln!("skipping describe_reads_columns_and_maps_types: duckdb not on PATH");
        return;
    }
    let db = temp_db("describe");
    create(
        &db,
        "CREATE TABLE food (id BIGINT, name VARCHAR, price DECIMAL(9,2), added TIMESTAMP);",
    );

    let cols = describe(&db, "food").unwrap();
    assert_eq!(
        cols,
        vec![
            ColumnTypeInfo {
                name: "id".into(),
                dict_type: DictType::Number,
                duckdb_type: "BIGINT".into()
            },
            ColumnTypeInfo {
                name: "name".into(),
                dict_type: DictType::String,
                duckdb_type: "VARCHAR".into()
            },
            ColumnTypeInfo {
                name: "price".into(),
                dict_type: DictType::Number,
                duckdb_type: "DECIMAL(9,2)".into()
            },
            ColumnTypeInfo {
                name: "added".into(),
                dict_type: DictType::Datetime,
                duckdb_type: "TIMESTAMP".into()
            },
        ]
    );
}

#[test]
fn column_types_projects_to_name_and_dict_string() {
    if !duckdb_available() {
        eprintln!("skipping column_types_projects_to_name_and_dict_string: duckdb not on PATH");
        return;
    }
    let db = temp_db("coltypes");
    create(&db, "CREATE TABLE t (a INTEGER, b BOOLEAN);");

    let pairs = column_types(&db, "t").unwrap();
    assert_eq!(
        pairs,
        vec![
            ("a".to_string(), "number".to_string()),
            ("b".to_string(), "boolean".to_string()),
        ]
    );
}

#[test]
fn missing_table_is_a_cli_error() {
    if !duckdb_available() {
        eprintln!("skipping missing_table_is_a_cli_error: duckdb not on PATH");
        return;
    }
    let db = temp_db("missing");
    create(&db, "CREATE TABLE t (a INTEGER);");

    let err = describe(&db, "nope").unwrap_err();
    assert!(matches!(err, DuckdbError::Cli { .. }), "got {err:?}");
}
