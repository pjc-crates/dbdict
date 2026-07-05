//! Tests for the native real-database reader: `read_schema` opens a duckdb
//! file read-only and returns every relation with its canonical column types.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use duckdb::Connection;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A unique temp path for one test's database file.
fn temp_db_file() -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "dbdict-duckdb-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("warehouse.duckdb")
}

#[test]
fn reads_every_relation_with_canonical_types() {
    let file = temp_db_file();
    // the connection is scoped so the database is flushed and closed before
    // the reader opens it
    {
        let conn = Connection::open(&file).expect("create db file");
        conn.execute_batch(
            "CREATE TABLE trades (
                qty BIGINT,
                price DECIMAL(12,2),
                home STRUCT(city VARCHAR, postcode INTEGER)
             );
             CREATE TABLE orders (id BIGINT);
             CREATE VIEW order_ids AS SELECT id FROM orders;",
        )
        .expect("create fixture schema");
    }

    let schema = dbdict_duckdb::read_schema(&file).expect("read schema");

    // relations come back alphabetically, views included — a dictionary
    // table may legitimately be backed by a view
    let names: Vec<&str> = schema.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["order_ids", "orders", "trades"]);

    // canonical types, byte-for-byte as DESCRIBE spells them
    let trades = &schema[2];
    assert_eq!(
        trades.columns,
        vec![
            ("qty".to_string(), "BIGINT".to_string()),
            ("price".to_string(), "DECIMAL(12,2)".to_string()),
            (
                "home".to_string(),
                "STRUCT(city VARCHAR, postcode INTEGER)".to_string()
            ),
        ]
    );
    assert_eq!(
        schema[0].columns,
        vec![("id".to_string(), "BIGINT".to_string())]
    );
}

#[test]
fn missing_database_file_is_an_error() {
    let file = temp_db_file(); // never created
    let err = dbdict_duckdb::read_schema(&file).expect_err("must not invent a database");
    assert!(!err.is_empty(), "the error must carry duckdb's reason");
}

#[test]
fn read_is_readonly_so_the_database_is_never_created_or_mutated() {
    let file = temp_db_file(); // never created
    let _ = dbdict_duckdb::read_schema(&file);
    // a read-write open would have created the file as an empty database
    assert!(
        !file.exists(),
        "read_schema must not create the database file"
    );
}
