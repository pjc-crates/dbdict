//! End-to-end round-trip tests: a real dictionary YAML validated against a
//! real duckdb database file through `dbdict::validate_meta` and the native
//! backend. These are the phase's acceptance scenarios; the fine-grained
//! behaviour lives in core's fake-backend tests and this crate's unit tests.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use dbdict::{Status, validate_meta};
use dbdict_duckdb::NativeDuckdb;
use duckdb::Connection;
use indoc::indoc;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A unique temp dir holding one test's dictionary + database pair.
fn temp_dir() -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "dbdict-e2e-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Create `warehouse.duckdb` in `dir` with the given schema DDL.
fn write_db(dir: &Path, ddl: &str) {
    let conn = Connection::open(dir.join("warehouse.duckdb")).expect("create db file");
    conn.execute_batch(ddl).expect("create fixture schema");
}

/// Write the dictionary and return its path.
fn write_dict(dir: &Path, yaml: &str) -> PathBuf {
    let path = dir.join("dbdict.yaml");
    std::fs::write(&path, yaml).unwrap();
    path
}

/// The standard dictionary: typedefs (compounding, struct, enum) and one
/// `trades` table using them alongside native type expressions.
const DICT: &str = indoc! {r#"
    $version: "0.2.0"
    $learn_more: https://github.com/pjc-crates/dbdict
    typedef:
      money: DECIMAL(12, 2)
      address: STRUCT(city VARCHAR, postcode INTEGER)
      mood: ENUM('happy', 'sad')
    source:
      duckdb:
        file: warehouse.duckdb
    tables:
      - name: trades
        columns:
          - name: qty
            type: BIGINT
          - name: price
            type: money
          - name: home
            type: address
          - name: feeling
            type: mood
"#};

/// The database schema that matches [`DICT`] exactly (spelled natively — the
/// round-trip must equate it with the dictionary's aliases).
const MATCHING_DDL: &str = "CREATE TABLE trades (
    qty BIGINT,
    price DECIMAL(12,2),
    home STRUCT(city VARCHAR, postcode INTEGER),
    feeling ENUM('happy', 'sad')
);";

#[test]
fn clean_match_validates_ok() {
    let dir = temp_dir();
    write_db(&dir, MATCHING_DDL);
    let dict = write_dict(&dir, DICT);

    let problems = validate_meta(&dict, None, &NativeDuckdb);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
}

#[test]
fn identifier_case_differences_still_match() {
    let dir = temp_dir();
    // duckdb identifiers are case-insensitive but case-preserving: the database
    // stores CamelCase names, the dictionary uses lowercase. they name the same
    // objects, so this must validate cleanly — no spurious M02/M03/M06/M07
    write_db(
        &dir,
        "CREATE TABLE Trades (
            Qty BIGINT,
            Price DECIMAL(12,2),
            Home STRUCT(city VARCHAR, postcode INTEGER),
            Feeling ENUM('happy', 'sad')
        );",
    );
    let dict = write_dict(&dir, DICT); // lowercase trades/qty/price/home/feeling

    let problems = validate_meta(&dict, None, &NativeDuckdb);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
}

#[test]
fn empty_database_reports_the_dictionary_table_as_missing() {
    let dir = temp_dir();
    // a real, openable database that simply holds no relations
    write_db(
        &dir,
        "CREATE TABLE placeholder (x INTEGER); DROP TABLE placeholder;",
    );
    let dict = write_dict(&dir, DICT);

    let problems = validate_meta(&dict, None, &NativeDuckdb);
    assert_eq!(problems.status(), Status::Error);
    // exactly one M06 for the single documented `trades` table; no M02 per
    // column (the table itself is missing, so its columns aren't chased)
    assert!(
        matches!(
            problems.items.as_slice(),
            [dbdict::Problem { code: Some(code), .. }] if *code == "M06"
        ),
        "got {:?}",
        problems.items
    );
}

#[test]
fn struct_field_type_difference_is_an_exact_m01() {
    let dir = temp_dir();
    // `home.postcode` is VARCHAR in the database, INTEGER in the dictionary
    write_db(
        &dir,
        "CREATE TABLE trades (
            qty BIGINT,
            price DECIMAL(12,2),
            home STRUCT(city VARCHAR, postcode VARCHAR),
            feeling ENUM('happy', 'sad')
        );",
    );
    let dict = write_dict(&dir, DICT);

    let problems = validate_meta(&dict, None, &NativeDuckdb);
    assert_eq!(problems.status(), Status::Error);
    assert_eq!(problems.items.len(), 1, "got {:?}", problems.items);
    let problem = &problems.items[0];
    assert_eq!(problem.code, Some("M01"));
    // the diff is the exact canonical STRUCT spelling, both sides
    assert!(
        problem
            .message
            .contains("STRUCT(city VARCHAR, postcode INTEGER)")
            && problem
                .message
                .contains("STRUCT(city VARCHAR, postcode VARCHAR)"),
        "got {:?}",
        problem.message
    );
}

#[test]
fn dropped_documented_column_is_m02() {
    let dir = temp_dir();
    // the database lost the `feeling` column
    write_db(
        &dir,
        "CREATE TABLE trades (
            qty BIGINT,
            price DECIMAL(12,2),
            home STRUCT(city VARCHAR, postcode INTEGER)
        );",
    );
    let dict = write_dict(&dir, DICT);

    let problems = validate_meta(&dict, None, &NativeDuckdb);
    assert_eq!(problems.status(), Status::Error);
    assert_eq!(problems.items.len(), 1, "got {:?}", problems.items);
    assert_eq!(problems.items[0].code, Some("M02"));
}

#[test]
fn undocumented_database_column_is_m03() {
    let dir = temp_dir();
    write_db(
        &dir,
        "CREATE TABLE trades (
            qty BIGINT,
            price DECIMAL(12,2),
            home STRUCT(city VARCHAR, postcode INTEGER),
            feeling ENUM('happy', 'sad'),
            venue VARCHAR
        );",
    );
    let dict = write_dict(&dir, DICT);

    let problems = validate_meta(&dict, None, &NativeDuckdb);
    assert_eq!(
        problems.status(),
        Status::Warning,
        "got {:?}",
        problems.items
    );
    assert_eq!(problems.items.len(), 1, "got {:?}", problems.items);
    let problem = &problems.items[0];
    assert_eq!(problem.code, Some("M03"));
    assert!(
        problem.message.contains("venue") && problem.message.contains("VARCHAR"),
        "got {:?}",
        problem.message
    );
}

#[test]
fn cyclic_typedef_reports_m08_with_duckdbs_reason() {
    let dir = temp_dir();
    write_db(&dir, MATCHING_DDL);
    let dict = write_dict(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-crates/dbdict
            typedef:
              cyc_a: cyc_b
              cyc_b: cyc_a
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                columns:
                  - name: qty
                    type: BIGINT
        "#},
    );

    let problems = validate_meta(&dict, None, &NativeDuckdb);
    assert_eq!(problems.status(), Status::Error);
    let m08: Vec<_> = problems
        .items
        .iter()
        .filter(|p| p.code == Some("M08"))
        .collect();
    assert_eq!(
        m08.len(),
        2,
        "both cycle members report, got {:?}",
        problems.items
    );
    // duckdb's own reason, located at the typedef in the dictionary source
    assert!(
        m08[0].message.contains("does not exist"),
        "got {:?}",
        m08[0].message
    );
    assert!(
        m08.iter().all(|p| p.location(&problems.source).is_some()),
        "M08 must be span-located"
    );
    // this slimmed-down dictionary documents only `qty`, so the database's
    // other columns legitimately warn as M03 — but every *error* must be M08
    assert!(
        problems
            .items
            .iter()
            .all(|p| p.code == Some("M08") || p.code == Some("M03")),
        "got {:?}",
        problems.items
    );
}

#[test]
fn missing_database_file_is_m05_at_the_source() {
    let dir = temp_dir();
    // no database file written at all
    let dict = write_dict(&dir, DICT);

    let problems = validate_meta(&dict, None, &NativeDuckdb);
    assert_eq!(problems.status(), Status::Error);
    assert_eq!(problems.items.len(), 1, "got {:?}", problems.items);
    let problem = &problems.items[0];
    assert_eq!(problem.code, Some("M05"));
    assert!(
        problem.location(&problems.source).is_some(),
        "M05 must point at the dictionary's source entry"
    );
}
