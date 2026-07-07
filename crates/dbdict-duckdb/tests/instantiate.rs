//! Tests for scratch instantiation: the dictionary's typedefs and typed
//! columns are created in an in-memory duckdb and DESCRIBEd back, giving the
//! canonical *expected* side of the round-trip.
//!
//! The `DataDict` fixtures are built by hand (with placeholder spans) rather
//! than parsed from YAML — lowering is core's business, already covered by
//! core's own tests; these tests pin the duckdb behaviour.
//! (the builder helpers below are copied into tests/expand.rs — keep the two
//! in step)

use std::sync::atomic::{AtomicU32, Ordering};

use dbdict::model::{Column, DataDict, Format, Spanned, Table, Typedef};
use dbdict::rich::InstantiateFailure;
use dbdict_duckdb::instantiate;
use quarto_source_map::SourceInfo;

static COUNTER: AtomicU32 = AtomicU32::new(0);

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
fn typed_columns_describe_to_canonical_types() {
    // `big` compounds on `money`, and is declared FIRST: creation order must
    // not matter (the fixpoint retries until everything resolvable resolves)
    let d = dict(
        vec![
            typedef("big", "money[]"),
            typedef("money", "DECIMAL(18, 4)"),
            typedef("address", "STRUCT(city VARCHAR, postcode INTEGER)"),
        ],
        vec![table(
            "trades",
            Vec::new(),
            vec![
                column("qty", Some("BIGINT")),
                column("price", Some("money")),
                column("prices", Some("big")),
                column("home", Some("address")),
                column("note", None), // untyped: makes no claim, absent
            ],
        )],
    );

    let result = instantiate(&d);

    assert!(result.failures.is_empty(), "got {:?}", result.failures);
    assert_eq!(
        result.tables,
        vec![vec![
            col("qty", "BIGINT"),
            col("price", "DECIMAL(18,4)"),
            col("prices", "DECIMAL(18,4)[]"),
            col("home", "STRUCT(city VARCHAR, postcode INTEGER)"),
        ]]
    );
}

#[test]
fn table_scoped_typedef_shadows_the_global() {
    let d = dict(
        vec![typedef("money", "DECIMAL(18, 4)")],
        vec![
            table(
                "trades",
                vec![typedef("money", "DECIMAL(12, 2)")],
                vec![column("price", Some("money"))],
            ),
            table("orders", Vec::new(), vec![column("price", Some("money"))]),
        ],
    );

    let result = instantiate(&d);

    assert!(result.failures.is_empty(), "got {:?}", result.failures);
    assert_eq!(
        result.tables,
        vec![
            // `trades` sees its own money
            vec![col("price", "DECIMAL(12,2)")],
            // `orders` still sees the global one
            vec![col("price", "DECIMAL(18,4)")],
        ]
    );
}

#[test]
fn cyclic_and_unknown_typedefs_stall_and_report() {
    let d = dict(
        vec![
            typedef("ok_type", "VARCHAR"),
            typedef("cyc_a", "cyc_b"),
            typedef("cyc_b", "cyc_a"),
            typedef("dangling", "NO_SUCH_TYPE"),
        ],
        vec![table(
            "trades",
            Vec::new(),
            vec![column("name", Some("ok_type"))],
        )],
    );

    let result = instantiate(&d);

    // the good typedef still serves the table
    assert_eq!(result.tables, vec![vec![col("name", "VARCHAR")]]);
    // the stalled group reports each member, with duckdb's own reason
    let mut failed: Vec<usize> = result
        .failures
        .iter()
        .map(|f| match f {
            InstantiateFailure::Typedef {
                table: None,
                index,
                error,
            } => {
                assert!(!error.is_empty());
                *index
            }
            other => panic!("expected global typedef failures, got {other:?}"),
        })
        .collect();
    failed.sort();
    assert_eq!(failed, vec![1, 2, 3], "got {:?}", result.failures);
}

#[test]
fn rejected_column_type_reports_and_spares_its_neighbours() {
    let d = dict(
        Vec::new(),
        vec![table(
            "trades",
            Vec::new(),
            vec![
                column("qty", Some("BIGINT")),
                column("status", Some("NO_SUCH_TYPE")),
            ],
        )],
    );

    let result = instantiate(&d);

    // the good column still gets an expected type
    assert_eq!(result.tables, vec![vec![col("qty", "BIGINT")]]);
    assert!(
        matches!(
            result.failures.as_slice(),
            [InstantiateFailure::Column { table: 0, column: 1, error }] if error.contains("NO_SUCH_TYPE")
        ),
        "got {:?}",
        result.failures
    );
}

#[test]
fn scoped_typedef_failure_is_reported_against_its_table() {
    let d = dict(
        Vec::new(),
        vec![table(
            "trades",
            vec![typedef("broken", "NO_SUCH_TYPE")],
            vec![column("qty", Some("BIGINT"))],
        )],
    );

    let result = instantiate(&d);

    assert_eq!(result.tables, vec![vec![col("qty", "BIGINT")]]);
    assert!(
        matches!(
            result.failures.as_slice(),
            [InstantiateFailure::Typedef { table: Some(0), index: 0, error }] if error.contains("NO_SUCH_TYPE")
        ),
        "got {:?}",
        result.failures
    );
}

#[test]
fn global_typedef_failures_are_not_duplicated_per_table() {
    // the broken global fails once (globally), not once more per table
    let d = dict(
        vec![typedef("broken", "NO_SUCH_TYPE")],
        vec![
            table("trades", Vec::new(), vec![column("qty", Some("BIGINT"))]),
            table("orders", Vec::new(), vec![column("id", Some("BIGINT"))]),
        ],
    );

    let result = instantiate(&d);

    assert_eq!(
        result.failures.len(),
        1,
        "one failure for one broken typedef, got {:?}",
        result.failures
    );
}

#[test]
fn malformed_type_with_top_level_comma_is_a_column_failure() {
    // a legitimate type never has an unparenthesised top-level comma; one that
    // does would make `probe` a multi-column table, so its extra columns must
    // not leak into the expected side as phantom columns — it is a failure
    let d = dict(
        Vec::new(),
        vec![table(
            "trades",
            Vec::new(),
            vec![
                column("qty", Some("BIGINT")),
                column("bad", Some("INTEGER, sneaky INTEGER")),
            ],
        )],
    );

    let result = instantiate(&d);

    // only the good column's canonical type survives; no `sneaky` phantom
    assert_eq!(result.tables, vec![vec![col("qty", "BIGINT")]]);
    assert!(
        matches!(
            result.failures.as_slice(),
            [InstantiateFailure::Column {
                table: 0,
                column: 1,
                ..
            }]
        ),
        "got {:?}",
        result.failures
    );
}

#[test]
fn type_expression_cannot_reach_the_filesystem() {
    // a shared dictionary is untrusted input; a type expression that smuggles
    // an ATTACH must not touch the filesystem when instantiated in the scratch
    // database (external access is disabled on the scratch connection)
    let mut marker = std::env::temp_dir();
    marker.push(format!(
        "dbdict-attack-marker-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&marker);
    let evil = format!(
        "BIGINT); ATTACH '{}' AS pwn; CREATE TABLE pwn.t (y INTEGER",
        marker.display()
    );
    let d = dict(
        Vec::new(),
        vec![table(
            "trades",
            Vec::new(),
            vec![column("qty", Some(&evil))],
        )],
    );

    let result = instantiate(&d);

    assert!(
        !marker.exists(),
        "a type expression must not create a file on disk"
    );
    // and it is reported as a column failure rather than silently swallowed
    assert!(
        matches!(
            result.failures.as_slice(),
            [InstantiateFailure::Column {
                table: 0,
                column: 0,
                ..
            }]
        ),
        "got {:?}",
        result.failures
    );
}

#[test]
fn table_with_no_typed_columns_instantiates_empty() {
    let d = dict(
        Vec::new(),
        vec![table("notes", Vec::new(), vec![column("body", None)])],
    );

    let result = instantiate(&d);

    assert_eq!(result.tables, vec![Vec::new()]);
    assert!(result.failures.is_empty());
}
