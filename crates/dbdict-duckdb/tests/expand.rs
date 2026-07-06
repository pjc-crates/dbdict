//! Tests for typedef expansion: each alias is created in a scratch duckdb and
//! probed back, giving the canonical spelling the `resolve` CLI command prints.
//!
//! Like the instantiate tests, the `DataDict` fixtures are built by hand with
//! placeholder spans — these tests pin the duckdb behaviour, not the lowering.
//! (the builder helpers below are copied from tests/instantiate.rs — keep the
//! two in step)

use dbdict::model::{Column, DataDict, Format, Spanned, Table, Typedef};
use dbdict_duckdb::expand_typedefs;
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
        tables,
        relationships: Vec::new(),
    }
}

#[test]
fn global_typedefs_expand_to_canonical_types() {
    // `big` compounds on `money` and is declared first: like instantiation,
    // expansion must not care about declaration order
    let d = dict(
        vec![
            typedef("big", "money[]"),
            typedef("money", "DECIMAL(18, 4)"),
            typedef("address", "STRUCT(city VARCHAR, postcode INTEGER)"),
        ],
        Vec::new(),
    );

    let expansions = expand_typedefs(&d);

    // document order is preserved, whatever order the fixpoint resolved in
    let got: Vec<(Option<&str>, &str, &str)> = expansions
        .iter()
        .map(|e| {
            (
                e.table.as_deref(),
                e.name.as_str(),
                e.expansion.as_deref().expect("expansion succeeds"),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (None, "big", "DECIMAL(18,4)[]"),
            (None, "money", "DECIMAL(18,4)"),
            (None, "address", "STRUCT(city VARCHAR, postcode INTEGER)"),
        ]
    );
}

#[test]
fn table_scoped_typedefs_expand_in_their_table_context() {
    // `trades` shadows the global `money`; its scoped list also compounds on
    // the shadowing definition
    let d = dict(
        vec![typedef("money", "DECIMAL(18, 4)")],
        vec![
            table(
                "trades",
                vec![
                    typedef("money", "DECIMAL(12, 2)"),
                    typedef("prices", "money[]"),
                ],
                vec![column("price", Some("money"))],
            ),
            // no scoped typedefs: contributes no entries
            table("orders", Vec::new(), vec![column("price", Some("money"))]),
        ],
    );

    let expansions = expand_typedefs(&d);

    let got: Vec<(Option<&str>, &str, &str)> = expansions
        .iter()
        .map(|e| {
            (
                e.table.as_deref(),
                e.name.as_str(),
                e.expansion.as_deref().expect("expansion succeeds"),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            // the global keeps its own expansion, unshadowed
            (None, "money", "DECIMAL(18,4)"),
            // the scoped entries expand against the shadowing definition
            (Some("trades"), "money", "DECIMAL(12,2)"),
            (Some("trades"), "prices", "DECIMAL(12,2)[]"),
        ]
    );
}

#[test]
fn shadowed_dependency_reshapes_a_global_in_table_context() {
    // `a` is NOT shadowed by `trades`, but its dependency `intish` is —
    // validation instantiates `trades` with `a` = VARCHAR, so the expansion
    // report must show that too (skipping it would contradict validate-meta)
    let d = dict(
        vec![
            typedef("intish", "INTEGER"),
            typedef("a", "intish"),
            typedef("b", "VARCHAR"), // unaffected by the shadowing
        ],
        vec![table(
            "trades",
            vec![typedef("intish", "VARCHAR")],
            vec![column("c", Some("a"))],
        )],
    );

    let expansions = expand_typedefs(&d);

    let got: Vec<(Option<&str>, &str, &str)> = expansions
        .iter()
        .map(|e| {
            (
                e.table.as_deref(),
                e.name.as_str(),
                e.expansion.as_deref().expect("expansion succeeds"),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (None, "intish", "INTEGER"),
            (None, "a", "INTEGER"),
            (None, "b", "VARCHAR"),
            // in trades' context: the reshaped global first (effective order
            // puts unshadowed globals before the scoped list), then the
            // scoped shadow itself; `b` doesn't change, so it isn't echoed
            (Some("trades"), "a", "VARCHAR"),
            (Some("trades"), "intish", "VARCHAR"),
        ]
    );
}

#[test]
fn stalled_scoped_typedef_error_lands_on_the_right_entry() {
    // the stalled-entry lookup is keyed by position in the *effective* list
    // (unshadowed globals first, then the scoped ones) — this pins that a
    // scoped failure is matched at its effective position, not its position
    // in table.typedefs
    let d = dict(
        vec![typedef("money", "DECIMAL(18, 4)")],
        vec![table(
            "trades",
            vec![
                typedef("ok_scoped", "VARCHAR"),
                typedef("dangling", "NO_SUCH_TYPE"),
            ],
            Vec::new(),
        )],
    );

    let expansions = expand_typedefs(&d);

    assert_eq!(expansions.len(), 3);
    assert_eq!(expansions[0].expansion.as_deref(), Ok("DECIMAL(18,4)"));
    assert_eq!(expansions[1].name, "ok_scoped");
    assert_eq!(expansions[1].expansion.as_deref(), Ok("VARCHAR"));
    assert_eq!(expansions[2].name, "dangling");
    let err = expansions[2]
        .expansion
        .as_ref()
        .expect_err("a dangling scoped alias cannot expand");
    assert!(err.contains("NO_SUCH_TYPE"), "got {err:?}");
}

#[test]
fn broken_typedefs_carry_duckdbs_error_and_spare_the_rest() {
    let d = dict(
        vec![
            typedef("ok_type", "VARCHAR"),
            typedef("dangling", "NO_SUCH_TYPE"),
        ],
        Vec::new(),
    );

    let expansions = expand_typedefs(&d);

    assert_eq!(expansions.len(), 2);
    assert_eq!(expansions[0].expansion.as_deref(), Ok("VARCHAR"));
    let err = expansions[1]
        .expansion
        .as_ref()
        .expect_err("a dangling alias cannot expand");
    assert!(err.contains("NO_SUCH_TYPE"), "got {err:?}");
}

#[test]
fn expr_is_carried_as_written() {
    // the CLI prints the declared expression next to its expansion, so the
    // original (non-canonical) spelling must survive
    let d = dict(vec![typedef("money", "decimal(18, 4)")], Vec::new());

    let expansions = expand_typedefs(&d);

    assert_eq!(expansions[0].expr, "decimal(18, 4)");
    assert_eq!(expansions[0].expansion.as_deref(), Ok("DECIMAL(18,4)"));
}

#[test]
fn dictionary_without_typedefs_expands_to_nothing() {
    let d = dict(
        Vec::new(),
        vec![table("t", Vec::new(), vec![column("a", Some("BIGINT"))])],
    );

    assert!(expand_typedefs(&d).is_empty());
}
