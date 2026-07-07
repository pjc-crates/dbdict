//! Unit tests for the generation-plan builder: table order, row counts,
//! per-column roles, and the refusal paths (legacy, cycles, unresolved or
//! ambiguous foreign keys, capacity, cardinality, range joins).
//!
//! `DataDict` fixtures are built by hand with placeholder spans, mirroring
//! crates/dbdict-ddl/tests/generate.rs — lowering is core's business.

use std::collections::HashMap;

use dbdict::join_expr::JoinExpr;
use dbdict::model::{
    Cardinality, Column, Constraint, DataDict, Format, Relationship, Spanned, Table,
};
use dbdict_dummy_data::{DummyDataError, GenerateOptions, Role, plan};
use quarto_source_map::SourceInfo;

fn spanned<T>(value: T) -> Spanned<T> {
    Spanned::new(value, SourceInfo::for_test())
}

fn column(name: &str, col_type: &str, constraints: &[Constraint]) -> Column {
    Column {
        name: spanned(name.to_string()),
        label: None,
        constraints: constraints.iter().map(|c| spanned(*c)).collect(),
        col_type: Some(spanned(col_type.to_string())),
        values: None,
        range: None,
        examples: None,
        units: None,
        time_zone: None,
    }
}

fn table(name: &str, columns: Vec<Column>) -> Table {
    Table {
        name: spanned(name.to_string()),
        label: None,
        typedefs: Vec::new(),
        columns,
        source: None,
        description: None,
        details: None,
    }
}

fn relationship(cardinality: Cardinality, join: &str) -> Relationship {
    Relationship {
        cardinality: spanned(cardinality),
        join_text: spanned(join.to_string()),
        join: Some(JoinExpr::parse(join).expect("test join parses")),
        conflicts: Vec::new(),
    }
}

fn dict(tables: Vec<Table>, relationships: Vec<Relationship>) -> DataDict {
    DataDict {
        format: Format::Rich,
        typedefs: Vec::new(),
        source: None,
        extensions: Vec::new(),
        tables,
        relationships,
    }
}

/// the customers/orders pair used by several tests: orders carries a
/// required fk to customers' pk, and is deliberately declared *first* so
/// ordering tests can see the topological flip
fn customers_orders() -> DataDict {
    dict(
        vec![
            table(
                "orders",
                vec![
                    column("id", "INTEGER", &[Constraint::PrimaryKey]),
                    column(
                        "customer_id",
                        "INTEGER",
                        &[Constraint::ForeignKey, Constraint::Required],
                    ),
                    column("note", "VARCHAR", &[]),
                ],
            ),
            table(
                "customers",
                vec![
                    column("id", "INTEGER", &[Constraint::PrimaryKey]),
                    column("email", "VARCHAR", &[Constraint::Unique]),
                    column("note", "VARCHAR", &[]),
                ],
            ),
        ],
        vec![relationship(
            Cardinality::ManyToOne,
            "orders.customer_id = customers.id",
        )],
    )
}

#[test]
fn legacy_dictionary_is_refused() {
    let mut d = dict(Vec::new(), Vec::new());
    d.format = Format::Legacy;
    let err = plan(&d, &GenerateOptions::default()).unwrap_err();
    assert!(matches!(err, DummyDataError::LegacyUnsupported));
}

#[test]
fn tables_are_ordered_fk_targets_first() {
    let p = plan(&customers_orders(), &GenerateOptions::default()).expect("plans");
    let order: Vec<&str> = p.tables.iter().map(|t| t.table.as_str()).collect();
    assert_eq!(order, ["customers", "orders"]);
}

#[test]
fn independent_tables_keep_document_order() {
    let d = dict(
        vec![
            table("c", vec![column("x", "INTEGER", &[])]),
            table("a", vec![column("x", "INTEGER", &[])]),
            table("b", vec![column("x", "INTEGER", &[])]),
        ],
        Vec::new(),
    );
    let p = plan(&d, &GenerateOptions::default()).expect("plans");
    let order: Vec<&str> = p.tables.iter().map(|t| t.table.as_str()).collect();
    assert_eq!(order, ["c", "a", "b"]);
}

#[test]
fn row_counts_use_global_default_and_per_table_override() {
    let opts = GenerateOptions {
        rows: 7,
        table_rows: HashMap::from([("customers".to_string(), 3)]),
        ..GenerateOptions::default()
    };
    let p = plan(&customers_orders(), &opts).expect("plans");
    let rows: HashMap<&str, u64> = p
        .tables
        .iter()
        .map(|t| (t.table.as_str(), t.rows))
        .collect();
    assert_eq!(rows["customers"], 3);
    assert_eq!(rows["orders"], 7);
}

#[test]
fn unknown_table_in_overrides_is_refused() {
    let opts = GenerateOptions {
        table_rows: HashMap::from([("nope".to_string(), 3)]),
        ..GenerateOptions::default()
    };
    let err = plan(&customers_orders(), &opts).unwrap_err();
    assert!(
        matches!(err, DummyDataError::UnknownTableOverride { ref table } if table == "nope"),
        "got: {err:?}"
    );
}

#[test]
fn roles_cover_pk_unique_fk_and_plain_fill() {
    let p = plan(&customers_orders(), &GenerateOptions::default()).expect("plans");

    let customers = &p.tables[0];
    // pk: unique by construction, never null
    assert_eq!(customers.columns[0].column, "id");
    assert_eq!(customers.columns[0].role, Role::IndexedUnique);
    assert!(!customers.columns[0].nullable);
    // unique but optional: still indexed-unique, may be null
    assert_eq!(customers.columns[1].column, "email");
    assert_eq!(customers.columns[1].role, Role::IndexedUnique);
    assert!(customers.columns[1].nullable);
    // unconstrained: plain fill
    assert_eq!(customers.columns[2].column, "note");
    assert_eq!(customers.columns[2].role, Role::PlainFill);
    assert!(customers.columns[2].nullable);

    let orders = &p.tables[1];
    // required fk: draw from the target pk, not injective, never null
    assert_eq!(orders.columns[1].column, "customer_id");
    assert_eq!(
        orders.columns[1].role,
        Role::FkDraw {
            target_table: "customers".to_string(),
            target_column: "id".to_string(),
            injective: false,
        }
    );
    assert!(!orders.columns[1].nullable);
}

#[test]
fn composite_pk_columns_are_each_indexed_unique() {
    let d = dict(
        vec![table(
            "pairs",
            vec![
                column("a", "INTEGER", &[Constraint::PrimaryKey]),
                column("b", "INTEGER", &[Constraint::PrimaryKey]),
            ],
        )],
        Vec::new(),
    );
    let p = plan(&d, &GenerateOptions::default()).expect("plans");
    for col in &p.tables[0].columns {
        assert_eq!(col.role, Role::IndexedUnique);
        assert!(!col.nullable);
    }
}

#[test]
fn fk_cycle_is_refused() {
    let d = dict(
        vec![
            table(
                "a",
                vec![
                    column("id", "INTEGER", &[Constraint::PrimaryKey]),
                    column("b_id", "INTEGER", &[Constraint::ForeignKey]),
                ],
            ),
            table(
                "b",
                vec![
                    column("id", "INTEGER", &[Constraint::PrimaryKey]),
                    column("a_id", "INTEGER", &[Constraint::ForeignKey]),
                ],
            ),
        ],
        vec![
            relationship(Cardinality::ManyToOne, "a.b_id = b.id"),
            relationship(Cardinality::ManyToOne, "b.a_id = a.id"),
        ],
    );
    let err = plan(&d, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(err, DummyDataError::ForeignKeyCycle { ref tables }
            if tables == &["a".to_string(), "b".to_string()]),
        "got: {err:?}"
    );
}

#[test]
fn self_referencing_fk_is_refused() {
    let d = dict(
        vec![table(
            "employees",
            vec![
                column("id", "INTEGER", &[Constraint::PrimaryKey]),
                column("manager_id", "INTEGER", &[Constraint::ForeignKey]),
            ],
        )],
        vec![relationship(
            Cardinality::ManyToOne,
            "employees.manager_id = employees.id",
        )],
    );
    let err = plan(&d, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(err, DummyDataError::ForeignKeyCycle { ref tables }
            if tables == &["employees".to_string()]),
        "got: {err:?}"
    );
}

#[test]
fn unresolved_fk_is_refused() {
    // fk constraint but no relationship pairs it with any primary key
    let d = dict(
        vec![table(
            "orders",
            vec![column("customer_id", "INTEGER", &[Constraint::ForeignKey])],
        )],
        Vec::new(),
    );
    let err = plan(&d, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(err, DummyDataError::UnresolvedForeignKey { ref table, ref column }
            if table == "orders" && column == "customer_id"),
        "got: {err:?}"
    );
}

#[test]
fn fk_with_two_distinct_targets_is_refused() {
    let d = dict(
        vec![
            table(
                "orders",
                vec![column("ref_id", "INTEGER", &[Constraint::ForeignKey])],
            ),
            table(
                "customers",
                vec![column("id", "INTEGER", &[Constraint::PrimaryKey])],
            ),
            table(
                "suppliers",
                vec![column("id", "INTEGER", &[Constraint::PrimaryKey])],
            ),
        ],
        vec![
            relationship(Cardinality::ManyToOne, "orders.ref_id = customers.id"),
            relationship(Cardinality::ManyToOne, "orders.ref_id = suppliers.id"),
        ],
    );
    let err = plan(&d, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(err, DummyDataError::AmbiguousForeignKey { ref table, ref column, .. }
            if table == "orders" && column == "ref_id"),
        "got: {err:?}"
    );
}

/// users/profiles one-to-one: the fk column is itself unique, so the draw
/// must be injective
fn users_profiles() -> DataDict {
    dict(
        vec![
            table(
                "users",
                vec![column("id", "INTEGER", &[Constraint::PrimaryKey])],
            ),
            table(
                "profiles",
                vec![
                    column("id", "INTEGER", &[Constraint::PrimaryKey]),
                    column(
                        "user_id",
                        "INTEGER",
                        &[
                            Constraint::ForeignKey,
                            Constraint::Unique,
                            Constraint::Required,
                        ],
                    ),
                ],
            ),
        ],
        vec![relationship(
            Cardinality::OneToOne,
            "profiles.user_id = users.id",
        )],
    )
}

#[test]
fn unique_fk_column_gets_an_injective_draw() {
    let p = plan(&users_profiles(), &GenerateOptions::default()).expect("plans");
    let profiles = p.tables.iter().find(|t| t.table == "profiles").unwrap();
    assert_eq!(
        profiles.columns[1].role,
        Role::FkDraw {
            target_table: "users".to_string(),
            target_column: "id".to_string(),
            injective: true,
        }
    );
}

#[test]
fn injective_fk_with_too_few_target_rows_is_refused() {
    // 5 unique draws from a 3-row target cannot all be distinct
    let opts = GenerateOptions {
        table_rows: HashMap::from([("users".to_string(), 3), ("profiles".to_string(), 5)]),
        ..GenerateOptions::default()
    };
    let err = plan(&users_profiles(), &opts).unwrap_err();
    assert!(
        matches!(err, DummyDataError::InjectiveFkExceedsTarget { ref table, ref column, rows, target_rows, .. }
            if table == "profiles" && column == "user_id" && rows == 5 && target_rows == 3),
        "got: {err:?}"
    );
}

#[test]
fn fk_draw_from_an_empty_target_is_refused() {
    let opts = GenerateOptions {
        table_rows: HashMap::from([("customers".to_string(), 0)]),
        ..GenerateOptions::default()
    };
    let err = plan(&customers_orders(), &opts).unwrap_err();
    assert!(
        matches!(err, DummyDataError::EmptyFkTarget { ref table, ref column, ref target_table }
            if table == "orders" && column == "customer_id" && target_table == "customers"),
        "got: {err:?}"
    );
}

#[test]
fn many_to_one_with_a_non_unique_one_side_is_refused() {
    // customers.email is not declared unique, so nothing guarantees an
    // orders row matches at most one customers row
    let d = dict(
        vec![
            table("orders", vec![column("customer_ref", "VARCHAR", &[])]),
            table("customers", vec![column("email", "VARCHAR", &[])]),
        ],
        vec![relationship(
            Cardinality::ManyToOne,
            "orders.customer_ref = customers.email",
        )],
    );
    let err = plan(&d, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(err, DummyDataError::CardinalityUnsatisfiable { ref one_table, .. }
            if one_table == "customers"),
        "got: {err:?}"
    );
}

#[test]
fn many_to_one_with_a_unique_one_side_is_accepted() {
    let d = dict(
        vec![
            table("orders", vec![column("customer_ref", "VARCHAR", &[])]),
            table(
                "customers",
                vec![column("email", "VARCHAR", &[Constraint::Unique])],
            ),
        ],
        vec![relationship(
            Cardinality::ManyToOne,
            "orders.customer_ref = customers.email",
        )],
    );
    plan(&d, &GenerateOptions::default()).expect("a unique one side satisfies d05");
}

#[test]
fn one_to_one_requires_both_sides_unique() {
    // a.x is unique but b.y is not: the b-side "one" declaration is
    // unsatisfiable by construction
    let d = dict(
        vec![
            table("a", vec![column("x", "INTEGER", &[Constraint::Unique])]),
            table("b", vec![column("y", "INTEGER", &[])]),
        ],
        vec![relationship(Cardinality::OneToOne, "a.x = b.y")],
    );
    let err = plan(&d, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(err, DummyDataError::CardinalityUnsatisfiable { ref one_table, .. }
            if one_table == "b"),
        "got: {err:?}"
    );
}

#[test]
fn range_join_is_refused_until_phase_5() {
    let d = dict(
        vec![
            table("events", vec![column("ts", "INTEGER", &[])]),
            table(
                "windows",
                vec![column("lo", "INTEGER", &[Constraint::Unique])],
            ),
        ],
        vec![relationship(
            Cardinality::ManyToOne,
            "events.ts >= windows.lo",
        )],
    );
    let err = plan(&d, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(err, DummyDataError::RangeJoinUnsupported { .. }),
        "got: {err:?}"
    );
}

#[test]
fn unparsed_join_is_refused() {
    let d = dict(
        vec![
            table("a", vec![column("x", "INTEGER", &[])]),
            table("b", vec![column("y", "INTEGER", &[])]),
        ],
        vec![Relationship {
            cardinality: spanned(Cardinality::ManyToOne),
            join_text: spanned("???".to_string()),
            join: None,
            conflicts: Vec::new(),
        }],
    );
    let err = plan(&d, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(err, DummyDataError::JoinUnparsed { .. }),
        "got: {err:?}"
    );
}

#[test]
fn null_fraction_outside_zero_to_one_is_refused() {
    let opts = GenerateOptions {
        null_fraction: 1.5,
        ..GenerateOptions::default()
    };
    let err = plan(&customers_orders(), &opts).unwrap_err();
    assert!(
        matches!(err, DummyDataError::NullFractionOutOfRange { .. }),
        "got: {err:?}"
    );
}
