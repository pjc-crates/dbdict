//! Integration tests for the data level on rich (0.2.0) documents: the
//! value-level checks (`D##`) run as queries against the dictionary's
//! database, driven through a fake [`DuckdbBackend`].
//!
//! Like `rich_meta.rs`, the fake returns canned results so these tests pin
//! down *which* problems are raised for *which* counts, without a duckdb
//! build. The real query SQL is proven by `dbdict-duckdb`'s own tests.

mod common;
use common::{temp_dir, write_yaml};

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use dbdict::model::DataDict;
use dbdict::rich::{DuckdbBackend, Instantiated, TableSchema, TypeCategory};
use dbdict::{Problem, ProblemKind, Status, validate_data};
use indoc::indoc;

/// a `(column, canonical type)` pair, as `DESCRIBE` would report it
fn col(name: &str, canonical_type: &str) -> (String, String) {
    (name.to_string(), canonical_type.to_string())
}

/// A canned backend for the data level. Meta-level canned results as in
/// `rich_meta.rs`, plus per-column null counts and a per-table duplicate-key
/// count for the data queries. Queries actually issued are recorded so tests
/// can assert what was (and was not) asked of the database.
struct FakeDuckdb {
    instantiated: Instantiated,
    db: Result<Vec<TableSchema>, String>,
    /// `"table.column"` → null count returned by `count_nulls`
    null_counts: HashMap<String, usize>,
    /// duplicate-key count returned by `count_duplicate_keys` for any table
    dup_key_count: usize,
    /// `"table.column"` → duplicate count returned by `count_duplicate_values`
    dup_value_counts: HashMap<String, usize>,
    /// every `count_duplicate_keys` call: `(table, key columns)`
    dup_calls: RefCell<Vec<(String, Vec<String>)>>,
    /// every `count_nulls` call: `"table.column"`
    null_calls: RefCell<Vec<String>>,
    /// every `count_duplicate_values` call: `"table.column"`
    value_calls: RefCell<Vec<String>>,
}

impl FakeDuckdb {
    /// a fake whose data is clean (no nulls, no duplicate keys)
    fn clean(instantiated: Instantiated, db: Result<Vec<TableSchema>, String>) -> Self {
        FakeDuckdb {
            instantiated,
            db,
            null_counts: HashMap::new(),
            dup_key_count: 0,
            dup_value_counts: HashMap::new(),
            dup_calls: RefCell::new(Vec::new()),
            null_calls: RefCell::new(Vec::new()),
            value_calls: RefCell::new(Vec::new()),
        }
    }
}

impl DuckdbBackend for FakeDuckdb {
    fn instantiate(&self, _dict: &DataDict) -> Instantiated {
        self.instantiated.clone()
    }

    fn read_schema(&self, _db_file: &Path) -> Result<Vec<TableSchema>, String> {
        self.db.clone()
    }

    fn classify(&self, _canonical_type: &str) -> TypeCategory {
        TypeCategory::Other
    }

    fn count_nulls(&self, _db_file: &Path, table: &str, column: &str) -> Result<usize, String> {
        let key = format!("{table}.{column}");
        self.null_calls.borrow_mut().push(key.clone());
        Ok(self.null_counts.get(&key).copied().unwrap_or(0))
    }

    fn count_duplicate_keys(
        &self,
        _db_file: &Path,
        table: &str,
        key_columns: &[String],
    ) -> Result<usize, String> {
        self.dup_calls
            .borrow_mut()
            .push((table.to_string(), key_columns.to_vec()));
        Ok(self.dup_key_count)
    }

    fn count_duplicate_values(
        &self,
        _db_file: &Path,
        table: &str,
        column: &str,
    ) -> Result<usize, String> {
        let key = format!("{table}.{column}");
        self.value_calls.borrow_mut().push(key.clone());
        Ok(self.dup_value_counts.get(&key).copied().unwrap_or(0))
    }
}

/// The standard fixture: `trades` with a required `qty`, a plain `note`, and
/// a single-column primary key `id`, sourcing `warehouse.duckdb`.
fn write_trades_dict(dir: &Path) -> PathBuf {
    write_yaml(
        dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: http://data-dict.tidyverse.org/
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: qty
                    type: BIGINT
                    constraints: [required]
                  - name: note
                    type: VARCHAR
        "#},
    )
}

fn trades_expected() -> Vec<(String, String)> {
    vec![
        col("id", "BIGINT"),
        col("qty", "BIGINT"),
        col("note", "VARCHAR"),
    ]
}

fn trades_db() -> Result<Vec<TableSchema>, String> {
    Ok(vec![TableSchema {
        name: "trades".to_string(),
        columns: trades_expected(),
    }])
}

fn instantiated() -> Instantiated {
    Instantiated {
        tables: vec![trades_expected()],
        failures: Vec::new(),
    }
}

// --- happy path -------------------------------------------------------------

/// clean data on a matching schema passes: the transitional "rich data level
/// not built" pre-flight is gone
#[test]
fn clean_rich_data_passes() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    let backend = FakeDuckdb::clean(instantiated(), trades_db());

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
}

/// only constrained columns are queried for nulls: `id` (primary_key) and
/// `qty` (required) — never the unconstrained `note`
#[test]
fn only_required_columns_are_queried_for_nulls() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    let backend = FakeDuckdb::clean(instantiated(), trades_db());

    validate_data(&yaml, None, &backend);
    let calls = backend.null_calls.borrow();
    assert_eq!(
        *calls,
        vec!["trades.id".to_string(), "trades.qty".to_string()]
    );
}

// --- D01 ---------------------------------------------------------------------

#[test]
fn nulls_in_required_column_is_d01() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    let mut backend = FakeDuckdb::clean(instantiated(), trades_db());
    backend.null_counts.insert("trades.qty".to_string(), 3);

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), kind: ProblemKind::NullsInRequired { count: 3, rows }, .. }]
                if *code == "D01" && rows.is_empty()
        ),
        "got {:?}",
        problems.items
    );
    let message = &problems.items[0].message;
    assert!(message.contains("3 null values"), "got {message:?}");
}

// --- D02 ---------------------------------------------------------------------

#[test]
fn duplicate_primary_key_is_d02() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    let mut backend = FakeDuckdb::clean(instantiated(), trades_db());
    backend.dup_key_count = 2;

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), kind: ProblemKind::DuplicateKey { count: 2 }, .. }]
                if *code == "D02"
        ),
        "got {:?}",
        problems.items
    );
}

/// several `primary_key` columns form one composite key: a single query over
/// the combination, and a single D02 when it has duplicates
#[test]
fn composite_primary_key_is_one_key() {
    let dir = temp_dir();
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: http://data-dict.tidyverse.org/
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: prices
                columns:
                  - name: sym
                    type: VARCHAR
                    constraints: [primary_key]
                  - name: day
                    type: DATE
                    constraints: [primary_key]
        "#},
    );
    let expected = vec![col("sym", "VARCHAR"), col("day", "DATE")];
    let mut backend = FakeDuckdb::clean(
        Instantiated {
            tables: vec![expected.clone()],
            failures: Vec::new(),
        },
        Ok(vec![TableSchema {
            name: "prices".to_string(),
            columns: expected,
        }]),
    );
    backend.dup_key_count = 1;

    let problems = validate_data(&yaml, None, &backend);
    let d02s: Vec<_> = problems
        .items
        .iter()
        .filter(|p| p.code == Some("D02"))
        .collect();
    assert_eq!(d02s.len(), 1, "got {:?}", problems.items);
    let calls = backend.dup_calls.borrow();
    assert_eq!(
        *calls,
        vec![(
            "prices".to_string(),
            vec!["sym".to_string(), "day".to_string()]
        )]
    );
}

/// a table with no `primary_key` columns is never queried for duplicates
#[test]
fn no_primary_key_no_duplicate_query() {
    let dir = temp_dir();
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: http://data-dict.tidyverse.org/
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: logs
                columns:
                  - name: line
                    type: VARCHAR
        "#},
    );
    let expected = vec![col("line", "VARCHAR")];
    let backend = FakeDuckdb::clean(
        Instantiated {
            tables: vec![expected.clone()],
            failures: Vec::new(),
        },
        Ok(vec![TableSchema {
            name: "logs".to_string(),
            columns: expected,
        }]),
    );

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
    assert!(backend.dup_calls.borrow().is_empty());
}

// --- D03 ---------------------------------------------------------------------

/// A fixture with an explicitly-`unique` column that is not the primary key:
/// `accounts` with pk `id` and unique `email`.
fn write_accounts_dict(dir: &Path) -> PathBuf {
    write_yaml(
        dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: http://data-dict.tidyverse.org/
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: accounts
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: email
                    type: VARCHAR
                    constraints: [unique]
        "#},
    )
}

fn accounts_expected() -> Vec<(String, String)> {
    vec![col("id", "BIGINT"), col("email", "VARCHAR")]
}

fn accounts_backend() -> FakeDuckdb {
    FakeDuckdb::clean(
        Instantiated {
            tables: vec![accounts_expected()],
            failures: Vec::new(),
        },
        Ok(vec![TableSchema {
            name: "accounts".to_string(),
            columns: accounts_expected(),
        }]),
    )
}

#[test]
fn duplicate_values_in_unique_column_is_d03() {
    let dir = temp_dir();
    let yaml = write_accounts_dict(&dir);
    let mut backend = accounts_backend();
    backend
        .dup_value_counts
        .insert("accounts.email".to_string(), 2);

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), kind: ProblemKind::DuplicateValues { count: 2 }, .. }]
                if *code == "D03"
        ),
        "got {:?}",
        problems.items
    );
    let message = &problems.items[0].message;
    assert!(message.contains("2 duplicated value"), "got {message:?}");
}

#[test]
fn clean_unique_column_is_queried_and_passes() {
    let dir = temp_dir();
    let yaml = write_accounts_dict(&dir);
    let backend = accounts_backend();

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
    // the unique column WAS checked — passing is a query result, not a skip
    assert_eq!(*backend.value_calls.borrow(), vec!["accounts.email"]);
}

/// a column that is by itself the whole primary key is D02's job: an explicit
/// `unique` on it must not trigger a second, identical D03 query
#[test]
fn sole_primary_key_column_is_not_double_checked() {
    let dir = temp_dir();
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: http://data-dict.tidyverse.org/
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: accounts
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key, unique]
        "#},
    );
    let expected = vec![col("id", "BIGINT")];
    let backend = FakeDuckdb::clean(
        Instantiated {
            tables: vec![expected.clone()],
            failures: Vec::new(),
        },
        Ok(vec![TableSchema {
            name: "accounts".to_string(),
            columns: expected,
        }]),
    );

    validate_data(&yaml, None, &backend);
    // D02 queried the key; D03 stayed out of it
    assert_eq!(backend.dup_calls.borrow().len(), 1);
    assert!(backend.value_calls.borrow().is_empty());
}

/// an explicit `unique` on a member of a *composite* key IS checked: D02's
/// tuple check deliberately does not imply per-column uniqueness
#[test]
fn composite_key_member_with_unique_is_checked() {
    let dir = temp_dir();
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: http://data-dict.tidyverse.org/
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: prices
                columns:
                  - name: sym
                    type: VARCHAR
                    constraints: [primary_key, unique]
                  - name: day
                    type: DATE
                    constraints: [primary_key]
        "#},
    );
    let expected = vec![col("sym", "VARCHAR"), col("day", "DATE")];
    let backend = FakeDuckdb::clean(
        Instantiated {
            tables: vec![expected.clone()],
            failures: Vec::new(),
        },
        Ok(vec![TableSchema {
            name: "prices".to_string(),
            columns: expected,
        }]),
    );

    validate_data(&yaml, None, &backend);
    assert_eq!(*backend.value_calls.borrow(), vec!["prices.sym"]);
}

/// `unique` alone does not imply `required`: the column is checked for
/// duplicates but never queried for nulls (D01's scope is unchanged)
#[test]
fn unique_without_required_is_not_null_queried() {
    let dir = temp_dir();
    let yaml = write_accounts_dict(&dir);
    let backend = accounts_backend();

    validate_data(&yaml, None, &backend);
    // only the pk `id` is null-queried; `email` (unique, optional) is not
    assert_eq!(*backend.null_calls.borrow(), vec!["accounts.id"]);
}

/// a failing D03 query is reported like any other lost check, not swallowed
#[test]
fn failing_duplicate_values_query_is_reported() {
    struct FailingValues {
        inner: FakeDuckdb,
    }
    impl DuckdbBackend for FailingValues {
        fn instantiate(&self, dict: &DataDict) -> Instantiated {
            self.inner.instantiate(dict)
        }
        fn read_schema(&self, db_file: &Path) -> Result<Vec<TableSchema>, String> {
            self.inner.read_schema(db_file)
        }
        fn classify(&self, canonical_type: &str) -> TypeCategory {
            self.inner.classify(canonical_type)
        }
        fn count_nulls(&self, db: &Path, table: &str, col: &str) -> Result<usize, String> {
            self.inner.count_nulls(db, table, col)
        }
        fn count_duplicate_keys(
            &self,
            db: &Path,
            table: &str,
            key_columns: &[String],
        ) -> Result<usize, String> {
            self.inner.count_duplicate_keys(db, table, key_columns)
        }
        fn count_duplicate_values(
            &self,
            _db: &Path,
            _table: &str,
            _column: &str,
        ) -> Result<usize, String> {
            Err("query interrupted".to_string())
        }
    }

    let dir = temp_dir();
    let yaml = write_accounts_dict(&dir);
    let backend = FailingValues {
        inner: accounts_backend(),
    };

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        problems
            .items
            .iter()
            .any(|p| matches!(p.kind, ProblemKind::UnreadableSource)
                && p.message.contains("query interrupted")),
        "got {:?}",
        problems.items
    );
}

// --- failure modes ------------------------------------------------------------

/// a data query the backend cannot answer is reported, not swallowed
#[test]
fn failing_data_query_is_reported() {
    struct FailingNulls {
        inner: FakeDuckdb,
    }
    impl DuckdbBackend for FailingNulls {
        fn instantiate(&self, dict: &DataDict) -> Instantiated {
            self.inner.instantiate(dict)
        }
        fn read_schema(&self, db_file: &Path) -> Result<Vec<TableSchema>, String> {
            self.inner.read_schema(db_file)
        }
        fn classify(&self, canonical_type: &str) -> TypeCategory {
            self.inner.classify(canonical_type)
        }
        fn count_nulls(&self, _db: &Path, _table: &str, _col: &str) -> Result<usize, String> {
            Err("query interrupted".to_string())
        }
        fn count_duplicate_keys(
            &self,
            db: &Path,
            table: &str,
            key_columns: &[String],
        ) -> Result<usize, String> {
            self.inner.count_duplicate_keys(db, table, key_columns)
        }
        fn count_duplicate_values(
            &self,
            db: &Path,
            table: &str,
            column: &str,
        ) -> Result<usize, String> {
            self.inner.count_duplicate_values(db, table, column)
        }
    }

    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    let backend = FailingNulls {
        inner: FakeDuckdb::clean(instantiated(), trades_db()),
    };

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        problems
            .items
            .iter()
            .any(|p| matches!(p.kind, ProblemKind::UnreadableSource)
                && p.message.contains("query interrupted")),
        "got {:?}",
        problems.items
    );
}

/// a dictionary table missing from the database gets its M06 from the meta
/// level, and no data queries are attempted against it
#[test]
fn missing_table_is_not_queried() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    let backend = FakeDuckdb::clean(instantiated(), Ok(Vec::new()));

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        problems
            .items
            .iter()
            .any(|p| matches!(p.kind, ProblemKind::MissingTable)),
        "got {:?}",
        problems.items
    );
    assert!(backend.null_calls.borrow().is_empty());
    assert!(backend.dup_calls.borrow().is_empty());
}
