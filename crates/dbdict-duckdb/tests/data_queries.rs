//! Tests for the native data-level queries: `count_nulls` (D01),
//! `count_duplicate_keys` (D02), and `count_duplicate_values` (D03) against
//! a real duckdb database file.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use dbdict::join_expr::JoinOp;
use dbdict::rich::OrientedConjunct;
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

// --- D05: count_overmatched_rows ---------------------------------------------

/// shorthand for one oriented conjunct, probe column first
fn conj(probe_column: &str, op: JoinOp, other_column: &str) -> OrientedConjunct {
    OrientedConjunct {
        probe_column: probe_column.to_string(),
        op,
        other_column: other_column.to_string(),
    }
}

/// the shorthand for the standard date-in-period join, probing events
fn within_period() -> Vec<OrientedConjunct> {
    vec![
        conj("date", JoinOp::Ge, "start"),
        conj("date", JoinOp::Le, "end"),
    ]
}

/// the motivating gap: overlapping ranges over-match a `many-to-one` range
/// join, which no constraint-based check can see
#[test]
fn overlapping_ranges_overmatch() {
    // periods 1 and 2 overlap through February; the 02-15 event falls in
    // both, the 01-10 event in period 1 alone — one over-matched row
    let file = build_db(
        "CREATE TABLE periods (id BIGINT, start DATE, \"end\" DATE);
         INSERT INTO periods VALUES
           (1, DATE '2020-01-01', DATE '2020-03-01'),
           (2, DATE '2020-02-01', DATE '2020-04-01');
         CREATE TABLE events (id BIGINT, date DATE);
         INSERT INTO events VALUES (1, DATE '2020-01-10'), (2, DATE '2020-02-15');",
    );
    assert_eq!(
        dbdict_duckdb::count_overmatched_rows(&file, "events", "periods", &within_period()),
        Ok(1)
    );
}

/// non-overlapping ranges keep every probe row at one match or none
#[test]
fn distinct_ranges_do_not_overmatch() {
    let file = build_db(
        "CREATE TABLE periods (id BIGINT, start DATE, \"end\" DATE);
         INSERT INTO periods VALUES
           (1, DATE '2020-01-01', DATE '2020-01-31'),
           (2, DATE '2020-02-01', DATE '2020-02-29');
         CREATE TABLE events (id BIGINT, date DATE);
         INSERT INTO events VALUES (1, DATE '2020-01-10'), (2, DATE '2020-02-15');",
    );
    assert_eq!(
        dbdict_duckdb::count_overmatched_rows(&file, "events", "periods", &within_period()),
        Ok(0)
    );
}

/// an equality join over-matches when the "one" side holds duplicates; rows
/// matching exactly one row — or none at all — are not violations
#[test]
fn duplicated_equality_matches_are_counted_and_zero_matches_pass() {
    // cat 1 exists twice; trades row (1) over-matches, row (2) matches one
    // row, row (99) matches none — one violation
    let file = build_db(
        "CREATE TABLE categories (id BIGINT);
         INSERT INTO categories VALUES (1), (1), (2);
         CREATE TABLE trades (cat_id BIGINT);
         INSERT INTO trades VALUES (1), (2), (99);",
    );
    assert_eq!(
        dbdict_duckdb::count_overmatched_rows(
            &file,
            "trades",
            "categories",
            &[conj("cat_id", JoinOp::Eq, "id")],
        ),
        Ok(1)
    );
}

/// a NULL join column satisfies no comparison, matches nothing, and passes
/// under the zero-match rule
#[test]
fn null_join_columns_match_nothing() {
    let file = build_db(
        "CREATE TABLE categories (id BIGINT);
         INSERT INTO categories VALUES (1), (1);
         CREATE TABLE trades (cat_id BIGINT);
         INSERT INTO trades VALUES (NULL), (NULL);",
    );
    assert_eq!(
        dbdict_duckdb::count_overmatched_rows(
            &file,
            "trades",
            "categories",
            &[conj("cat_id", JoinOp::Eq, "id")],
        ),
        Ok(0)
    );
}

/// several conjuncts are one ANDed predicate: a period that overlaps by date
/// but belongs to another calendar does not over-match
#[test]
fn multi_conjunct_predicates_are_anded() {
    let file = build_db(
        "CREATE TABLE periods (cal BIGINT, start DATE, \"end\" DATE);
         INSERT INTO periods VALUES
           (1, DATE '2020-01-01', DATE '2020-03-01'),
           (2, DATE '2020-02-01', DATE '2020-04-01');
         CREATE TABLE events (cal BIGINT, date DATE);
         INSERT INTO events VALUES (1, DATE '2020-02-15');",
    );
    // by date alone the event is in both periods, but the calendars differ
    assert_eq!(
        dbdict_duckdb::count_overmatched_rows(
            &file,
            "events",
            "periods",
            &[
                conj("cal", JoinOp::Eq, "cal"),
                conj("date", JoinOp::Ge, "start"),
                conj("date", JoinOp::Le, "end"),
            ],
        ),
        Ok(0)
    );
}

/// a self-join probes and counts the same table; the aliases keep the two
/// roles apart
#[test]
fn self_join_overmatches_are_counted() {
    // two employees share id 1: everyone reporting to 1 over-matches
    let file = build_db(
        "CREATE TABLE employees (id BIGINT, manager_id BIGINT);
         INSERT INTO employees VALUES (1, NULL), (1, NULL), (2, 1), (3, 2);",
    );
    assert_eq!(
        dbdict_duckdb::count_overmatched_rows(
            &file,
            "employees",
            "employees",
            &[conj("manager_id", JoinOp::Eq, "id")],
        ),
        Ok(1)
    );
}

#[test]
fn overmatched_rows_match_identifiers_case_insensitively() {
    let file = build_db(
        "CREATE TABLE Categories (Id BIGINT);
         INSERT INTO Categories VALUES (1), (1);
         CREATE TABLE Trades (Cat_Id BIGINT);
         INSERT INTO Trades VALUES (1);",
    );
    assert_eq!(
        dbdict_duckdb::count_overmatched_rows(
            &file,
            "trades",
            "categories",
            &[conj("cat_id", JoinOp::Eq, "id")],
        ),
        Ok(1)
    );
}

/// quoting keeps hostile names inert on both tables and every conjunct column
#[test]
fn overmatch_query_quotes_both_tables() {
    let file = build_db(
        r#"CREATE TABLE "cat""egories; DROP TABLE x" ("i""d" BIGINT);
           INSERT INTO "cat""egories; DROP TABLE x" VALUES (1), (1);
           CREATE TABLE "tra""des" ("cat""_id" BIGINT);
           INSERT INTO "tra""des" VALUES (1);"#,
    );
    assert_eq!(
        dbdict_duckdb::count_overmatched_rows(
            &file,
            r#"tra"des"#,
            r#"cat"egories; DROP TABLE x"#,
            &[conj(r#"cat"_id"#, JoinOp::Eq, r#"i"d"#)],
        ),
        Ok(1)
    );
}

#[test]
fn overmatched_rows_on_a_missing_column_is_an_error() {
    let file = build_db(
        "CREATE TABLE categories (id BIGINT);
         CREATE TABLE trades (cat_id BIGINT);",
    );
    let result = dbdict_duckdb::count_overmatched_rows(
        &file,
        "trades",
        "categories",
        &[conj("no_such", JoinOp::Eq, "id")],
    );
    assert!(result.is_err(), "got {result:?}");
}
