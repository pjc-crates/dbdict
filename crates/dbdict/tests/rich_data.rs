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

use dbdict::join_expr::JoinOp;
use dbdict::model::DataDict;
use dbdict::rich::{DuckdbBackend, Instantiated, OrientedConjunct, TableSchema, TypeCategory};
use dbdict::{Problem, ProblemKind, Status, validate_data};
use indoc::indoc;

/// render an operator the way the join text spells it, for call-log keys
fn op_str(op: JoinOp) -> &'static str {
    match op {
        JoinOp::Eq => "=",
        JoinOp::Ge => ">=",
        JoinOp::Le => "<=",
        JoinOp::Gt => ">",
        JoinOp::Lt => "<",
    }
}

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
    /// `"fk_table.fk_column->pk_table.pk_column"` → orphan count returned by
    /// `count_orphaned_values`
    orphan_counts: HashMap<String, usize>,
    /// `"probe_table->other_table"` → over-match count returned by
    /// `count_overmatched_rows`
    overmatch_counts: HashMap<String, usize>,
    /// when set, every `count_overmatched_rows` call fails with this reason
    overmatch_error: Option<String>,
    /// every `count_duplicate_keys` call: `(table, key columns)`
    dup_calls: RefCell<Vec<(String, Vec<String>)>>,
    /// every `count_nulls` call: `"table.column"`
    null_calls: RefCell<Vec<String>>,
    /// every `count_duplicate_values` call: `"table.column"`
    value_calls: RefCell<Vec<String>>,
    /// every `count_orphaned_values` call:
    /// `"fk_table.fk_column->pk_table.pk_column"`
    orphan_calls: RefCell<Vec<String>>,
    /// every `count_overmatched_rows` call, with its oriented conjuncts:
    /// `"probe_table->other_table: col<op>col,…"`
    overmatch_calls: RefCell<Vec<String>>,
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
            orphan_counts: HashMap::new(),
            overmatch_counts: HashMap::new(),
            overmatch_error: None,
            dup_calls: RefCell::new(Vec::new()),
            null_calls: RefCell::new(Vec::new()),
            value_calls: RefCell::new(Vec::new()),
            orphan_calls: RefCell::new(Vec::new()),
            overmatch_calls: RefCell::new(Vec::new()),
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

    fn count_orphaned_values(
        &self,
        _db_file: &Path,
        fk_table: &str,
        fk_column: &str,
        pk_table: &str,
        pk_column: &str,
    ) -> Result<usize, String> {
        let key = format!("{fk_table}.{fk_column}->{pk_table}.{pk_column}");
        self.orphan_calls.borrow_mut().push(key.clone());
        Ok(self.orphan_counts.get(&key).copied().unwrap_or(0))
    }

    fn count_overmatched_rows(
        &self,
        _db_file: &Path,
        probe_table: &str,
        other_table: &str,
        conjuncts: &[OrientedConjunct],
    ) -> Result<usize, String> {
        // the log keeps the oriented conjuncts so tests can assert both the
        // probe direction and the operator flip
        let rendered: Vec<String> = conjuncts
            .iter()
            .map(|c| format!("{}{}{}", c.probe_column, op_str(c.op), c.other_column))
            .collect();
        let key = format!("{probe_table}->{other_table}");
        self.overmatch_calls
            .borrow_mut()
            .push(format!("{key}: {}", rendered.join(",")));
        if let Some(reason) = &self.overmatch_error {
            return Err(reason.clone());
        }
        Ok(self.overmatch_counts.get(&key).copied().unwrap_or(0))
    }
}

/// The standard fixture: `trades` with a required `qty`, a plain `note`, and
/// a single-column primary key `id`, sourcing `warehouse.duckdb`.
fn write_trades_dict(dir: &Path) -> PathBuf {
    write_yaml(
        dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
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
            $learn_more: https://github.com/pjc-wspace/dbdict
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
            $learn_more: https://github.com/pjc-wspace/dbdict
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
            $learn_more: https://github.com/pjc-wspace/dbdict
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
            $learn_more: https://github.com/pjc-wspace/dbdict
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
            $learn_more: https://github.com/pjc-wspace/dbdict
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
        fn count_orphaned_values(
            &self,
            db: &Path,
            fk_table: &str,
            fk_col: &str,
            pk_table: &str,
            pk_col: &str,
        ) -> Result<usize, String> {
            self.inner
                .count_orphaned_values(db, fk_table, fk_col, pk_table, pk_col)
        }
        fn count_overmatched_rows(
            &self,
            db: &Path,
            probe_table: &str,
            other_table: &str,
            conjuncts: &[OrientedConjunct],
        ) -> Result<usize, String> {
            self.inner
                .count_overmatched_rows(db, probe_table, other_table, conjuncts)
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
        fn count_orphaned_values(
            &self,
            db: &Path,
            fk_table: &str,
            fk_col: &str,
            pk_table: &str,
            pk_col: &str,
        ) -> Result<usize, String> {
            self.inner
                .count_orphaned_values(db, fk_table, fk_col, pk_table, pk_col)
        }
        fn count_overmatched_rows(
            &self,
            db: &Path,
            probe_table: &str,
            other_table: &str,
            conjuncts: &[OrientedConjunct],
        ) -> Result<usize, String> {
            self.inner
                .count_overmatched_rows(db, probe_table, other_table, conjuncts)
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

// --- D04 ---------------------------------------------------------------------

/// The standard fk fixture: `trades.cat_id` is `foreign_key`, paired with the
/// `primary_key` column `categories.id` by an equality join.
fn write_fk_dict(dir: &Path) -> PathBuf {
    write_yaml(
        dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: cat_id
                    type: BIGINT
                    constraints: [foreign_key]
              - name: categories
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
            relationships:
              - join: trades.cat_id = categories.id
                cardinality: many-to-one
        "#},
    )
}

fn fk_trades_expected() -> Vec<(String, String)> {
    vec![col("id", "BIGINT"), col("cat_id", "BIGINT")]
}

fn fk_categories_expected() -> Vec<(String, String)> {
    vec![col("id", "BIGINT")]
}

fn fk_db() -> Result<Vec<TableSchema>, String> {
    Ok(vec![
        TableSchema {
            name: "trades".to_string(),
            columns: fk_trades_expected(),
        },
        TableSchema {
            name: "categories".to_string(),
            columns: fk_categories_expected(),
        },
    ])
}

fn fk_instantiated() -> Instantiated {
    Instantiated {
        tables: vec![fk_trades_expected(), fk_categories_expected()],
        failures: Vec::new(),
    }
}

#[test]
fn orphaned_fk_value_is_d04() {
    let dir = temp_dir();
    let yaml = write_fk_dict(&dir);
    let mut backend = FakeDuckdb::clean(fk_instantiated(), fk_db());
    backend
        .orphan_counts
        .insert("trades.cat_id->categories.id".to_string(), 2);

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), kind: ProblemKind::OrphanedValues { count: 2 }, .. }]
                if *code == "D04"
        ),
        "got {:?}",
        problems.items
    );
    // the message names the pk target so a column with several declared
    // targets carries tellable-apart problems
    let message = &problems.items[0].message;
    assert!(message.contains("2 orphaned values"), "got {message:?}");
    assert!(message.contains("categories.id"), "got {message:?}");
}

/// clean fk data passes, exactly the declared pair is queried, and the fk
/// column (constrained but not `required`) is never null-queried — D01's
/// scope is unchanged by `foreign_key`
#[test]
fn clean_fk_column_queries_the_declared_pair() {
    let dir = temp_dir();
    let yaml = write_fk_dict(&dir);
    let backend = FakeDuckdb::clean(fk_instantiated(), fk_db());

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
    assert_eq!(
        *backend.orphan_calls.borrow(),
        vec!["trades.cat_id->categories.id".to_string()]
    );
    assert_eq!(
        *backend.null_calls.borrow(),
        vec!["trades.id".to_string(), "categories.id".to_string()]
    );
}

/// a fk column paired with two primary keys is checked against each — every
/// declared pairing stands alone (as in SQL), one problem per violating pair
#[test]
fn every_declared_fk_pk_pair_is_checked() {
    let dir = temp_dir();
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: cat_id
                    type: BIGINT
                    constraints: [foreign_key]
              - name: categories
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
              - name: archive
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
            relationships:
              - join: trades.cat_id = categories.id
                cardinality: many-to-one
              - join: trades.cat_id = archive.id
                cardinality: many-to-one
        "#},
    );
    let archive_expected = vec![col("id", "BIGINT")];
    let mut backend = FakeDuckdb::clean(
        Instantiated {
            tables: vec![
                fk_trades_expected(),
                fk_categories_expected(),
                archive_expected.clone(),
            ],
            failures: Vec::new(),
        },
        Ok(vec![
            TableSchema {
                name: "trades".to_string(),
                columns: fk_trades_expected(),
            },
            TableSchema {
                name: "categories".to_string(),
                columns: fk_categories_expected(),
            },
            TableSchema {
                name: "archive".to_string(),
                columns: archive_expected,
            },
        ]),
    );
    backend
        .orphan_counts
        .insert("trades.cat_id->categories.id".to_string(), 1);
    backend
        .orphan_counts
        .insert("trades.cat_id->archive.id".to_string(), 3);

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert_eq!(
        *backend.orphan_calls.borrow(),
        vec![
            "trades.cat_id->categories.id".to_string(),
            "trades.cat_id->archive.id".to_string(),
        ]
    );
    let d04s: Vec<_> = problems
        .items
        .iter()
        .filter(|p| matches!(p.kind, ProblemKind::OrphanedValues { .. }))
        .collect();
    assert_eq!(d04s.len(), 2, "got {:?}", problems.items);
    assert!(
        d04s.iter().any(
            |p| matches!(p.kind, ProblemKind::OrphanedValues { count: 1 })
                && p.message.contains("categories.id")
        ),
        "got {d04s:?}"
    );
    assert!(
        d04s.iter().any(
            |p| matches!(p.kind, ProblemKind::OrphanedValues { count: 3 })
                && p.message.contains("archive.id")
        ),
        "got {d04s:?}"
    );
}

/// a range conjunct relates the fk column to the pk without referencing it:
/// no D04 query runs (and the spec level reports the fk as unresolved, S01)
#[test]
fn range_conjunct_does_not_pair_for_d04() {
    let dir = temp_dir();
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: cat_id
                    type: BIGINT
                    constraints: [foreign_key]
              - name: categories
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
            relationships:
              - join: trades.cat_id >= categories.id
                cardinality: many-to-one
        "#},
    );
    let backend = FakeDuckdb::clean(fk_instantiated(), fk_db());

    let problems = validate_data(&yaml, None, &backend);
    assert!(backend.orphan_calls.borrow().is_empty());
    assert!(
        !problems
            .items
            .iter()
            .any(|p| matches!(p.kind, ProblemKind::OrphanedValues { .. })),
        "got {:?}",
        problems.items
    );
}

/// a self-join fk (a hierarchy) queries the same table on both sides
#[test]
fn self_join_fk_queries_the_same_table() {
    let dir = temp_dir();
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: employees
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: manager_id
                    type: BIGINT
                    constraints: [foreign_key]
            relationships:
              - join: employees.manager_id = employees.id
                cardinality: many-to-one
        "#},
    );
    let expected = vec![col("id", "BIGINT"), col("manager_id", "BIGINT")];
    let backend = FakeDuckdb::clean(
        Instantiated {
            tables: vec![expected.clone()],
            failures: Vec::new(),
        },
        Ok(vec![TableSchema {
            name: "employees".to_string(),
            columns: expected,
        }]),
    );

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
    assert_eq!(
        *backend.orphan_calls.borrow(),
        vec!["employees.manager_id->employees.id".to_string()]
    );
}

/// the pk-side table absent from the database already has its M06: the pair
/// can't be queried, so D04 is skipped rather than double-reported
#[test]
fn missing_pk_table_is_not_queried_for_d04() {
    let dir = temp_dir();
    let yaml = write_fk_dict(&dir);
    let backend = FakeDuckdb::clean(
        fk_instantiated(),
        Ok(vec![TableSchema {
            name: "trades".to_string(),
            columns: fk_trades_expected(),
        }]),
    );

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
    assert!(backend.orphan_calls.borrow().is_empty());
}

/// the pk-side *column* absent from the database already has its M02: the
/// pair can't be queried, so D04 is skipped
#[test]
fn missing_pk_column_is_not_queried_for_d04() {
    let dir = temp_dir();
    let yaml = write_fk_dict(&dir);
    let backend = FakeDuckdb::clean(
        fk_instantiated(),
        Ok(vec![
            TableSchema {
                name: "trades".to_string(),
                columns: fk_trades_expected(),
            },
            TableSchema {
                name: "categories".to_string(),
                columns: vec![col("name", "VARCHAR")], // no `id`
            },
        ]),
    );

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(backend.orphan_calls.borrow().is_empty());
}

/// a failing D04 query is reported like any other lost check, not swallowed
#[test]
fn failing_orphan_query_is_reported() {
    struct FailingOrphans {
        inner: FakeDuckdb,
    }
    impl DuckdbBackend for FailingOrphans {
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
            db: &Path,
            table: &str,
            column: &str,
        ) -> Result<usize, String> {
            self.inner.count_duplicate_values(db, table, column)
        }
        fn count_orphaned_values(
            &self,
            _db: &Path,
            _fk_table: &str,
            _fk_col: &str,
            _pk_table: &str,
            _pk_col: &str,
        ) -> Result<usize, String> {
            Err("query interrupted".to_string())
        }
        fn count_overmatched_rows(
            &self,
            db: &Path,
            probe_table: &str,
            other_table: &str,
            conjuncts: &[OrientedConjunct],
        ) -> Result<usize, String> {
            self.inner
                .count_overmatched_rows(db, probe_table, other_table, conjuncts)
        }
    }

    let dir = temp_dir();
    let yaml = write_fk_dict(&dir);
    let backend = FailingOrphans {
        inner: FakeDuckdb::clean(fk_instantiated(), fk_db()),
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

// --- D05 ---------------------------------------------------------------------

/// The standard range-join fixture: `events.date` falls inside a period,
/// declared `many-to-one` — overlapping periods would violate it. `start` is
/// `unique` so S06's permissive range rule is satisfied and the diagnostics
/// isolate D05.
fn write_range_dict(dir: &Path) -> PathBuf {
    write_yaml(
        dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: events
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: date
                    type: DATE
              - name: periods
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: start
                    type: DATE
                    constraints: [unique]
                  - name: end
                    type: DATE
            relationships:
              - join: events.date >= periods.start AND events.date <= periods.end
                cardinality: many-to-one
        "#},
    )
}

fn range_events_expected() -> Vec<(String, String)> {
    vec![col("id", "BIGINT"), col("date", "DATE")]
}

fn range_periods_expected() -> Vec<(String, String)> {
    vec![
        col("id", "BIGINT"),
        col("start", "DATE"),
        col("end", "DATE"),
    ]
}

fn range_db() -> Result<Vec<TableSchema>, String> {
    Ok(vec![
        TableSchema {
            name: "events".to_string(),
            columns: range_events_expected(),
        },
        TableSchema {
            name: "periods".to_string(),
            columns: range_periods_expected(),
        },
    ])
}

fn range_instantiated() -> Instantiated {
    Instantiated {
        tables: vec![range_events_expected(), range_periods_expected()],
        failures: Vec::new(),
    }
}

/// over-matching rows violate the declared cardinality: `many-to-one` probes
/// the left ("many") side, and the range conjuncts cross the seam unflipped
#[test]
fn overmatching_rows_violate_declared_cardinality() {
    let dir = temp_dir();
    let yaml = write_range_dict(&dir);
    let mut backend = FakeDuckdb::clean(range_instantiated(), range_db());
    backend
        .overmatch_counts
        .insert("events->periods".to_string(), 2);

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), kind: ProblemKind::CardinalityViolation { count: 2 }, .. }]
                if *code == "D05"
        ),
        "got {:?}",
        problems.items
    );
    // the message names the declared cardinality and the over-matched side
    let message = &problems.items[0].message;
    assert!(message.contains("many-to-one"), "got {message:?}");
    assert!(message.contains("periods"), "got {message:?}");
    // probe = left table, conjuncts oriented probe-side first, ops unflipped
    assert_eq!(
        *backend.overmatch_calls.borrow(),
        vec!["events->periods: date>=start,date<=end".to_string()]
    );
}

/// `one-to-many` reads left-to-right: the left side is the "one" side, so
/// the *right* table's rows are probed for over-matching
#[test]
fn one_to_many_probes_the_right_side() {
    let dir = temp_dir();
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: categories
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
              - name: trades
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: cat_id
                    type: BIGINT
            relationships:
              - join: categories.id = trades.cat_id
                cardinality: one-to-many
        "#},
    );
    let categories_expected = vec![col("id", "BIGINT")];
    let trades_expected = vec![col("id", "BIGINT"), col("cat_id", "BIGINT")];
    let backend = FakeDuckdb::clean(
        Instantiated {
            tables: vec![categories_expected.clone(), trades_expected.clone()],
            failures: Vec::new(),
        },
        Ok(vec![
            TableSchema {
                name: "categories".to_string(),
                columns: categories_expected,
            },
            TableSchema {
                name: "trades".to_string(),
                columns: trades_expected,
            },
        ]),
    );

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
    // probe = right table; the equality conjunct reads probe-side first
    assert_eq!(
        *backend.overmatch_calls.borrow(),
        vec!["trades->categories: cat_id=id".to_string()]
    );
}

/// `one-to-one` checks both directions independently — two probes, flipped
/// operators on the second — and a single violating direction yields exactly
/// one problem naming the side that over-matches
#[test]
fn one_to_one_checks_both_directions() {
    let dir = temp_dir();
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: events
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: date
                    type: DATE
                    constraints: [unique]
              - name: periods
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: start
                    type: DATE
                    constraints: [unique]
                  - name: end
                    type: DATE
            relationships:
              - join: events.date >= periods.start AND events.date <= periods.end
                cardinality: one-to-one
        "#},
    );
    let events_expected = vec![col("id", "BIGINT"), col("date", "DATE")];
    let mut backend = FakeDuckdb::clean(
        Instantiated {
            tables: vec![events_expected.clone(), range_periods_expected()],
            failures: Vec::new(),
        },
        Ok(vec![
            TableSchema {
                name: "events".to_string(),
                columns: events_expected,
            },
            TableSchema {
                name: "periods".to_string(),
                columns: range_periods_expected(),
            },
        ]),
    );
    backend
        .overmatch_counts
        .insert("periods->events".to_string(), 3);

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    // both directions probed; the second flips each operator so the backend
    // still reads probe-side first (`p.start <= e.date` is `e.date >= p.start`)
    assert_eq!(
        *backend.overmatch_calls.borrow(),
        vec![
            "events->periods: date>=start,date<=end".to_string(),
            "periods->events: start<=date,end>=date".to_string(),
        ]
    );
    let d05s: Vec<_> = problems
        .items
        .iter()
        .filter(|p| matches!(p.kind, ProblemKind::CardinalityViolation { .. }))
        .collect();
    assert_eq!(d05s.len(), 1, "got {:?}", problems.items);
    assert!(
        matches!(d05s[0].kind, ProblemKind::CardinalityViolation { count: 3 }),
        "got {:?}",
        d05s[0]
    );
    // the violating direction probes `periods`, so `events` over-matches
    assert!(d05s[0].message.contains("events"), "got {:?}", d05s[0]);
}

/// a dictionary with no relationships never asks the cardinality question
#[test]
fn no_relationships_means_no_overmatch_queries() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    let backend = FakeDuckdb::clean(instantiated(), trades_db());

    validate_data(&yaml, None, &backend);
    assert!(backend.overmatch_calls.borrow().is_empty());
}

/// a join table absent from the database already has its M06: the join
/// can't be evaluated, so D05 is skipped rather than double-reported
#[test]
fn missing_join_table_skips_d05() {
    let dir = temp_dir();
    let yaml = write_range_dict(&dir);
    let backend = FakeDuckdb::clean(
        range_instantiated(),
        Ok(vec![TableSchema {
            name: "events".to_string(),
            columns: range_events_expected(),
        }]),
    );

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
    assert!(backend.overmatch_calls.borrow().is_empty());
}

/// a join *column* absent from the database already has its M02: the join
/// can't be evaluated, so D05 is skipped
#[test]
fn missing_join_column_skips_d05() {
    let dir = temp_dir();
    let yaml = write_range_dict(&dir);
    let backend = FakeDuckdb::clean(
        range_instantiated(),
        Ok(vec![
            TableSchema {
                name: "events".to_string(),
                columns: range_events_expected(),
            },
            TableSchema {
                name: "periods".to_string(),
                // no `end`
                columns: vec![col("id", "BIGINT"), col("start", "DATE")],
            },
        ]),
    );

    let problems = validate_data(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        problems
            .items
            .iter()
            .any(|p| matches!(p.kind, ProblemKind::MissingInData)),
        "got {:?}",
        problems.items
    );
    assert!(backend.overmatch_calls.borrow().is_empty());
}

/// a failing D05 query is reported like any other lost check, not swallowed
#[test]
fn failing_overmatch_query_is_reported() {
    let dir = temp_dir();
    let yaml = write_range_dict(&dir);
    let mut backend = FakeDuckdb::clean(range_instantiated(), range_db());
    backend.overmatch_error = Some("query interrupted".to_string());

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
