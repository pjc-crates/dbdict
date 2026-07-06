//! Tests for the native data-level queries: `count_nulls` (D01),
//! `count_duplicate_keys` (D02), and `count_duplicate_values` (D03) against
//! a real duckdb database file.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use duckdb::Connection;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A unique temp path for one test's database file.
fn temp_db_file() -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "dbdict-duckdb-data-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("warehouse.duckdb")
}

/// Build a database at a fresh temp path from one SQL batch, closed and
/// flushed before returning.
fn build_db(sql: &str) -> PathBuf {
    let file = temp_db_file();
    {
        let conn = Connection::open(&file).expect("create db file");
        conn.execute_batch(sql).expect("create fixture");
    }
    file
}

#[test]
fn counts_nulls_in_one_column() {
    let file = build_db(
        "CREATE TABLE trades (qty BIGINT, note VARCHAR);
         INSERT INTO trades VALUES (1, 'a'), (NULL, 'b'), (NULL, NULL);",
    );
    assert_eq!(dbdict_duckdb::count_nulls(&file, "trades", "qty"), Ok(2));
    assert_eq!(dbdict_duckdb::count_nulls(&file, "trades", "note"), Ok(1));
}

/// dictionary names may differ from the database's spelling only in case;
/// quoted identifiers still match, as duckdb folds identifier case
#[test]
fn identifiers_match_case_insensitively() {
    let file = build_db(
        "CREATE TABLE Trades (Qty BIGINT);
         INSERT INTO Trades VALUES (NULL);",
    );
    assert_eq!(dbdict_duckdb::count_nulls(&file, "trades", "qty"), Ok(1));
}

#[test]
fn missing_column_is_an_error_not_a_panic() {
    let file = build_db("CREATE TABLE trades (qty BIGINT);");
    let result = dbdict_duckdb::count_nulls(&file, "trades", "no_such");
    assert!(result.is_err(), "got {result:?}");
}

#[test]
fn counts_duplicated_single_column_keys() {
    // 7 appears twice and 9 three times: two distinct duplicated key values
    let file = build_db(
        "CREATE TABLE t (id BIGINT);
         INSERT INTO t VALUES (7), (7), (9), (9), (9), (1);",
    );
    assert_eq!(
        dbdict_duckdb::count_duplicate_keys(&file, "t", &["id".to_string()]),
        Ok(2)
    );
}

/// a composite key is duplicated only when the whole combination repeats —
/// repeats within one column alone are fine
#[test]
fn composite_keys_are_judged_as_a_whole() {
    let file = build_db(
        "CREATE TABLE px (sym VARCHAR, day DATE);
         INSERT INTO px VALUES
           ('AAPL', DATE '2026-01-01'),
           ('AAPL', DATE '2026-01-02'),
           ('MSFT', DATE '2026-01-01'),
           ('MSFT', DATE '2026-01-01');",
    );
    // sym and day each repeat, but only (MSFT, 2026-01-01) repeats as a pair
    assert_eq!(
        dbdict_duckdb::count_duplicate_keys(&file, "px", &["sym".to_string(), "day".to_string()]),
        Ok(1)
    );
}

#[test]
fn unique_keys_count_zero() {
    let file = build_db(
        "CREATE TABLE t (id BIGINT);
         INSERT INTO t VALUES (1), (2), (3);",
    );
    assert_eq!(
        dbdict_duckdb::count_duplicate_keys(&file, "t", &["id".to_string()]),
        Ok(0)
    );
}

#[test]
fn counts_duplicated_values_in_a_unique_column() {
    // 'a' appears twice and 'b' three times: two distinct duplicated values
    let file = build_db(
        "CREATE TABLE t (email VARCHAR);
         INSERT INTO t VALUES ('a'), ('a'), ('b'), ('b'), ('b'), ('c');",
    );
    assert_eq!(
        dbdict_duckdb::count_duplicate_values(&file, "t", "email"),
        Ok(2)
    );
}

/// the semantics lock for D03: repeated NULLs are NOT duplicates — SQL
/// UNIQUE treats NULLs as distinct, and an optional-but-unique column may
/// legitimately hold many of them (unlike D02, which counts NULL keys)
#[test]
fn repeated_nulls_are_not_duplicate_values() {
    let file = build_db(
        "CREATE TABLE t (email VARCHAR);
         INSERT INTO t VALUES (NULL), (NULL), (NULL), ('a');",
    );
    assert_eq!(
        dbdict_duckdb::count_duplicate_values(&file, "t", "email"),
        Ok(0)
    );
}

#[test]
fn duplicate_values_match_identifiers_case_insensitively() {
    let file = build_db(
        "CREATE TABLE Trades (Email VARCHAR);
         INSERT INTO Trades VALUES ('a'), ('a');",
    );
    assert_eq!(
        dbdict_duckdb::count_duplicate_values(&file, "trades", "email"),
        Ok(1)
    );
}

#[test]
fn duplicate_values_on_a_missing_column_is_an_error() {
    let file = build_db("CREATE TABLE t (email VARCHAR);");
    let result = dbdict_duckdb::count_duplicate_values(&file, "t", "no_such");
    assert!(result.is_err(), "got {result:?}");
}

/// quoting keeps a hostile name inert: an identifier with quotes and SQL in
/// it is just a name
#[test]
fn quoted_identifiers_are_inert() {
    let file = build_db(
        r#"CREATE TABLE "we""ird; DROP TABLE x" ("col""umn" BIGINT);
           INSERT INTO "we""ird; DROP TABLE x" VALUES (NULL);"#,
    );
    assert_eq!(
        dbdict_duckdb::count_nulls(&file, r#"we"ird; DROP TABLE x"#, r#"col"umn"#),
        Ok(1)
    );
}

#[test]
fn counts_orphaned_foreign_key_values() {
    // 5 (twice) and 7 have no match in categories: two distinct orphans
    let file = build_db(
        "CREATE TABLE categories (id BIGINT);
         INSERT INTO categories VALUES (1), (2);
         CREATE TABLE trades (cat_id BIGINT);
         INSERT INTO trades VALUES (1), (2), (5), (5), (7);",
    );
    assert_eq!(
        dbdict_duckdb::count_orphaned_values(&file, "trades", "cat_id", "categories", "id"),
        Ok(2)
    );
}

/// the D04 NULL-exclusion lock: a NULL foreign key means "no reference"
/// (SQL MATCH SIMPLE), not an orphan
#[test]
fn null_foreign_keys_are_not_orphans() {
    let file = build_db(
        "CREATE TABLE categories (id BIGINT);
         INSERT INTO categories VALUES (1);
         CREATE TABLE trades (cat_id BIGINT);
         INSERT INTO trades VALUES (1), (NULL), (NULL);",
    );
    assert_eq!(
        dbdict_duckdb::count_orphaned_values(&file, "trades", "cat_id", "categories", "id"),
        Ok(0)
    );
}

/// the anti-join lock: with `NOT IN`, a single NULL in the *primary key*
/// column makes the predicate NULL for every candidate and silently reports
/// zero orphans — the query must stay null-safe
#[test]
fn nulls_in_the_primary_key_column_do_not_mask_orphans() {
    let file = build_db(
        "CREATE TABLE categories (id BIGINT);
         INSERT INTO categories VALUES (1), (NULL);
         CREATE TABLE trades (cat_id BIGINT);
         INSERT INTO trades VALUES (1), (9);",
    );
    assert_eq!(
        dbdict_duckdb::count_orphaned_values(&file, "trades", "cat_id", "categories", "id"),
        Ok(1)
    );
}

/// a self-join fk (a hierarchy): the same table on both sides of the query
#[test]
fn self_join_orphans_are_counted() {
    // manager 99 does not exist as an id
    let file = build_db(
        "CREATE TABLE employees (id BIGINT, manager_id BIGINT);
         INSERT INTO employees VALUES (1, NULL), (2, 1), (3, 99);",
    );
    assert_eq!(
        dbdict_duckdb::count_orphaned_values(&file, "employees", "manager_id", "employees", "id"),
        Ok(1)
    );
}

#[test]
fn orphaned_values_match_identifiers_case_insensitively() {
    let file = build_db(
        "CREATE TABLE Categories (Id BIGINT);
         INSERT INTO Categories VALUES (1);
         CREATE TABLE Trades (Cat_Id BIGINT);
         INSERT INTO Trades VALUES (1), (9);",
    );
    assert_eq!(
        dbdict_duckdb::count_orphaned_values(&file, "trades", "cat_id", "categories", "id"),
        Ok(1)
    );
}

#[test]
fn orphaned_values_on_a_missing_column_is_an_error() {
    let file = build_db(
        "CREATE TABLE categories (id BIGINT);
         CREATE TABLE trades (cat_id BIGINT);",
    );
    let result =
        dbdict_duckdb::count_orphaned_values(&file, "trades", "no_such", "categories", "id");
    assert!(result.is_err(), "got {result:?}");
    let result =
        dbdict_duckdb::count_orphaned_values(&file, "trades", "cat_id", "categories", "no_such");
    assert!(result.is_err(), "got {result:?}");
}

/// quoting keeps hostile names inert on *both* sides of the anti-join
#[test]
fn orphan_query_quotes_both_tables() {
    let file = build_db(
        r#"CREATE TABLE "cat""egories; DROP TABLE x" ("i""d" BIGINT);
           INSERT INTO "cat""egories; DROP TABLE x" VALUES (1);
           CREATE TABLE "tra""des" ("cat""_id" BIGINT);
           INSERT INTO "tra""des" VALUES (1), (9);"#,
    );
    assert_eq!(
        dbdict_duckdb::count_orphaned_values(
            &file,
            r#"tra"des"#,
            r#"cat"_id"#,
            r#"cat"egories; DROP TABLE x"#,
            r#"i"d"#,
        ),
        Ok(1)
    );
}
