//! End-to-end tests for the generator: generate a database from a real
//! dictionary file, then let `dbdict::validate_data` — the same engine
//! users run — judge it. The validators are the oracle: a generated
//! database must pass every declared check (D01–D05) with zero problems.
//!
//! Fixtures are YAML on disk (not hand-built models) because
//! `validate_data` takes a dictionary *path* and loads it itself.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use dbdict::{Status, validate_data};
use dbdict_duckdb::NativeDuckdb;
use dbdict_dummy_data::GenerateOptions;
use dbdict_dummy_data_duckdb::{GenerateError, generate};
use indoc::indoc;

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A unique temp dir holding one test's dbdict.yaml; returns the dict path.
fn fixture(yaml: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "dbdict-dummy-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let dict_path = dir.join("dbdict.yaml");
    std::fs::write(&dict_path, yaml).unwrap();
    dict_path
}

/// Load a fixture dictionary, asserting it is spec-clean — a broken
/// fixture should fail loudly here, not confuse the test downstream.
fn load(dict_path: &std::path::Path) -> dbdict::model::DataDict {
    let (problems, dict) = dbdict::load_and_lower(dict_path).expect("fixture lowers");
    assert_eq!(
        problems.status(),
        Status::Ok,
        "fixture has spec problems:\n{}",
        problems.render().join("\n")
    );
    dict
}

/// The rich-surface fixture: typedefs (decimal, enum, struct), list and
/// fixed arrays, MAP, TIMESTAMP, a JSON column under a declared extension,
/// all four constraints, and a many-to-one relationship. `orders` is
/// declared *before* its fk target so generation must reorder the inserts.
const RICH_DICT: &str = indoc! {r#"
    $version: "0.2.0"
    $learn_more: https://github.com/pjc-wspace/dbdict
    typedef:
      money: DECIMAL(12, 2)
      side: ENUM('buy', 'sell')
      address: STRUCT(city VARCHAR, postcode INTEGER)
    duckdb:
      extensions:
        - json
    source:
      duckdb:
        file: data.duckdb
    tables:
      - name: orders
        columns:
          - name: id
            type: BIGINT
            constraints: [primary_key]
          - name: customer_id
            type: INTEGER
            constraints: [foreign_key, required]
          - name: amount
            type: money
            constraints: [required]
          - name: side
            type: side
          - name: placed_at
            type: TIMESTAMP
            constraints: [required]
          - name: tags
            type: VARCHAR[]
          - name: vec
            type: FLOAT[3]
          - name: attrs
            type: MAP(VARCHAR, INTEGER)
          - name: meta
            type: JSON
      - name: customers
        columns:
          - name: id
            type: INTEGER
            constraints: [primary_key]
          - name: email
            type: VARCHAR
            constraints: [unique]
          - name: home
            type: address
          - name: note
            type: VARCHAR
    relationships:
      - join: orders.customer_id = customers.id
        cardinality: many-to-one
"#};

#[test]
fn generated_database_passes_validate_data_end_to_end() {
    let dict_path = fixture(RICH_DICT);
    let dict = load(&dict_path);

    let generated = generate(&dict, &GenerateOptions::default()).expect("generates");

    // fk targets must be inserted first, whatever the document order says
    let script = &generated.script;
    let customers_at = script
        .find("INSERT INTO \"customers\"")
        .expect("customers insert present");
    let orders_at = script
        .find("INSERT INTO \"orders\"")
        .expect("orders insert present");
    assert!(
        customers_at < orders_at,
        "customers must be inserted before orders:\n{script}"
    );

    // write the database exactly where the dictionary's source points
    generated
        .write_db(&dict_path.parent().unwrap().join("data.duckdb"))
        .expect("writes");

    // the oracle: the shipped validators find nothing wrong
    let problems = validate_data(&dict_path, None, &NativeDuckdb);
    assert_eq!(
        problems.status(),
        Status::Ok,
        "generated database must pass validate-data:\n{}",
        problems.render().join("\n")
    );
}

#[test]
fn one_to_one_with_unique_fk_passes_validate_data() {
    let dict_path = fixture(indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: users
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
          - name: profiles
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: user_id
                type: INTEGER
                constraints: [foreign_key, unique, required]
        relationships:
          - join: profiles.user_id = users.id
            cardinality: one-to-one
    "#});
    let dict = load(&dict_path);

    let generated = generate(&dict, &GenerateOptions::default()).expect("generates");
    generated
        .write_db(&dict_path.parent().unwrap().join("data.duckdb"))
        .expect("writes");

    // one-to-one is probed in both directions by D05
    let problems = validate_data(&dict_path, None, &NativeDuckdb);
    assert_eq!(
        problems.status(),
        Status::Ok,
        "generated database must pass validate-data:\n{}",
        problems.render().join("\n")
    );
}

#[test]
fn same_seed_is_byte_identical_and_different_seed_differs() {
    let dict_path = fixture(RICH_DICT);
    let dict = load(&dict_path);

    let opts = GenerateOptions::default();
    let a = generate(&dict, &opts).expect("generates");
    let b = generate(&dict, &opts).expect("generates");
    assert_eq!(a.script, b.script, "same seed must be reproducible");

    let other = GenerateOptions {
        seed: 42,
        ..GenerateOptions::default()
    };
    let c = generate(&dict, &other).expect("generates");
    assert_ne!(
        a.script, c.script,
        "a different seed must change plain-fill values"
    );
}

#[test]
fn null_fraction_one_nulls_every_optional_value_and_still_validates() {
    let yaml = indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: t
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: note
                type: VARCHAR
    "#};

    let dict_path = fixture(yaml);
    let dict = load(&dict_path);
    let opts = GenerateOptions {
        rows: 5,
        null_fraction: 1.0,
        ..GenerateOptions::default()
    };
    let generated = generate(&dict, &opts).expect("generates");
    // every `note` value is NULL; `id` (required) never is
    assert_eq!(
        generated.script.matches("NULL").count(),
        5,
        "{}",
        generated.script
    );
    generated
        .write_db(&dict_path.parent().unwrap().join("data.duckdb"))
        .expect("writes");
    let problems = validate_data(&dict_path, None, &NativeDuckdb);
    assert_eq!(
        problems.status(),
        Status::Ok,
        "all-NULL optional column must still validate:\n{}",
        problems.render().join("\n")
    );

    // and with fraction 0.0 no NULL appears at all
    let opts = GenerateOptions {
        rows: 5,
        null_fraction: 0.0,
        ..GenerateOptions::default()
    };
    let generated = generate(&dict, &opts).expect("generates");
    assert_eq!(
        generated.script.matches("NULL").count(),
        0,
        "{}",
        generated.script
    );
}

#[test]
fn write_db_refuses_an_existing_file() {
    let dict_path = fixture(indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: t
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
    "#});
    let dict = load(&dict_path);
    let generated = generate(&dict, &GenerateOptions::default()).expect("generates");

    let out = dict_path.parent().unwrap().join("data.duckdb");
    std::fs::write(&out, b"precious bytes").unwrap();
    let err = generated.write_db(&out).unwrap_err();
    assert!(
        matches!(err, GenerateError::OutputExists { .. }),
        "got: {err:?}"
    );
    // and the file was not touched
    assert_eq!(std::fs::read(&out).unwrap(), b"precious bytes");
}

#[test]
fn unique_enum_with_more_rows_than_variants_is_refused() {
    let dict_path = fixture(indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: t
            columns:
              - name: flag
                type: ENUM('yes', 'no')
                constraints: [unique]
    "#});
    let dict = load(&dict_path);

    // 10 rows (the default), only 2 distinct enum values — refused up
    // front, before any rendering, not via a mid-render exhaustion
    let err = generate(&dict, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(
            err,
            GenerateError::UniqueCapacityTooSmall {
                ref table,
                ref column,
                capacity: 2,
                rows: 10,
            } if table == "t" && column == "flag"
        ),
        "got: {err:?}"
    );
}

#[test]
fn a_type_the_engine_rejects_is_refused_at_instantiation() {
    // INET needs the inet extension, which this build does not link
    let dict_path = fixture(indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: t
            columns:
              - name: addr
                type: INET
    "#});
    let dict = load(&dict_path);
    let err = generate(&dict, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(err, GenerateError::Instantiate { .. }),
        "got: {err:?}"
    );
}
