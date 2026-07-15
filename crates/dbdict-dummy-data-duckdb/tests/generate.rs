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

/// The `--sql` export is `Generated.script`; it must be a *complete* script
/// that rebuilds the database on its own — including LOADing any declared
/// extension. Regression guard: extension LOADs used to run inside `write_db`
/// and were absent from the exported script, so `--sql` on a dict declaring an
/// extension produced a script that could not reproduce the db by itself.
#[test]
fn exported_script_self_contains_extension_loads_and_reproduces_the_db() {
    // RICH_DICT declares the json extension (and a JSON column that needs it)
    let dict_path = fixture(RICH_DICT);
    let dict = load(&dict_path);
    let generated = generate(&dict, &GenerateOptions::default()).expect("generates");

    // this text assertion is the REAL regression guard: it fails on the pre-fix
    // code (which left the LOAD out of the exported script). json is statically
    // linked into this build (duckdb feature "bundled,json"), so its JSON type is
    // always available and no connection setting can force the explicit LOAD to
    // be required — verified empirically: with the LOAD fold removed the round-
    // trip below still passes. so only inspecting the emitted text catches a
    // dropped LOAD; the round-trip proves executability, not the LOAD's presence
    assert!(
        generated.script.contains("LOAD json;"),
        "exported script must LOAD its declared extension:\n{}",
        generated.script
    );

    // round-trip: execute the exported script — and nothing else — on a fresh
    // connection, proving it runs standalone (valid DDL/INSERTs, and a LOAD that
    // is a well-formed statement duckdb accepts). exactly what a user running the
    // `--sql` file by hand would do
    let conn = duckdb::Connection::open_in_memory().expect("in-memory db");
    conn.execute_batch(&generated.script)
        .expect("exported script must run standalone");
    let orders: i64 = conn
        .query_row("SELECT count(*) FROM orders", [], |r| r.get(0))
        .expect("orders table present and populated");
    // decoupled from the exact default row count: this test is about
    // self-containment, not how many rows the default happens to generate
    assert!(
        orders > 0,
        "round-trip db should be populated, got {orders}"
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

/// Range-join fixture skeleton: `windows` owns slots (`lo`/`hi`),
/// `events.at` probes them. Callers splice in the join line, cardinality
/// and any extra columns via plain string substitution to keep each
/// test's YAML readable at the call site.
///
/// `lo` is declared `unique` here to exercise D03 distinctness on a slot
/// bound — the declaration is honest (slot edges are `nth(3i)`, distinct by
/// construction). It is *not* required: as of the S06 range-join fix, a
/// range join needs no unique/`primary_key` bound at all (disjoint slots
/// give the at-most-one guarantee), which
/// [`many_to_one_range_join_with_non_unique_bound_validates`] pins down.
/// Some one-to-one tests also mark the probe column via `at_constraints`.
fn range_dict(
    extra_windows: &str,
    extra_events: &str,
    at_constraints: &str,
    join: &str,
    cardinality: &str,
) -> String {
    format!(
        r#"$version: "0.2.0"
$learn_more: https://github.com/pjc-wspace/dbdict
source:
  duckdb:
    file: data.duckdb
tables:
  - name: windows
    columns:
      - name: id
        type: INTEGER
        constraints: [primary_key]
      - name: lo
        type: TIMESTAMP
        constraints: [unique]
      - name: hi
        type: TIMESTAMP
{extra_windows}  - name: events
    columns:
      - name: id
        type: INTEGER
        constraints: [primary_key]
      - name: at
        type: TIMESTAMP
{at_constraints}{extra_events}relationships:
  - join: {join}
    cardinality: {cardinality}
"#
    )
}

/// Generate, write, and run the shipped validators — the shared happy-path
/// tail of every range-join oracle test.
fn assert_validates(dict_path: &std::path::Path, dict: &dbdict::model::DataDict) {
    let generated = generate(dict, &GenerateOptions::default()).expect("generates");
    generated
        .write_db(&dict_path.parent().unwrap().join("data.duckdb"))
        .expect("writes");
    let problems = validate_data(dict_path, None, &NativeDuckdb);
    assert_eq!(
        problems.status(),
        Status::Ok,
        "generated database must pass validate-data:\n{}",
        problems.render().join("\n")
    );
}

#[test]
fn many_to_one_range_join_passes_validate_data() {
    // the motivating D05 case: each event lands inside exactly one window
    let yaml = range_dict(
        "",
        "",
        "",
        "events.at >= windows.lo AND events.at <= windows.hi",
        "many-to-one",
    );
    let dict_path = fixture(&yaml);
    let dict = load(&dict_path);
    assert_validates(&dict_path, &dict);
}

#[test]
fn many_to_one_range_join_with_non_unique_bound_validates() {
    // the S06 fix in action end-to-end: neither bound column is unique or a
    // primary key, yet the dict validates (S06 exempts range joins) and the
    // generated database passes validate-data — disjoint slots satisfy D05
    // without any declared uniqueness
    let dict_path = fixture(indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: windows
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: lo
                type: TIMESTAMP
              - name: hi
                type: TIMESTAMP
          - name: events
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: at
                type: TIMESTAMP
        relationships:
          - join: events.at >= windows.lo AND events.at <= windows.hi
            cardinality: many-to-one
    "#});
    let dict = load(&dict_path);
    assert_validates(&dict_path, &dict);
}

#[test]
fn one_to_one_range_join_passes_validate_data() {
    // one-to-one is probed in both directions by D05: event i must own
    // window i exclusively (identity slot-owner draw)
    let yaml = range_dict(
        "",
        "",
        "        constraints: [unique]\n",
        "events.at >= windows.lo AND events.at <= windows.hi",
        "one-to-one",
    );
    let dict_path = fixture(&yaml);
    let dict = load(&dict_path);
    assert_validates(&dict_path, &dict);
}

#[test]
fn open_gt_lt_bounds_pass_validate_data() {
    // stride 3 exists exactly for this: the probe value sits strictly
    // between the slot edges, so open bounds match too
    let yaml = range_dict(
        "",
        "",
        "",
        "events.at > windows.lo AND events.at < windows.hi",
        "many-to-one",
    );
    let dict_path = fixture(&yaml);
    let dict = load(&dict_path);
    assert_validates(&dict_path, &dict);
}

#[test]
fn eq_conjunct_with_plain_fill_source_passes_validate_data() {
    // events.device copies the slot owner's plain-fill value, so the eq
    // conjunct agrees with the range conjuncts about which window matches
    let yaml = range_dict(
        "      - name: device\n        type: INTEGER\n",
        "      - name: device\n        type: INTEGER\n",
        "",
        "events.at >= windows.lo AND events.at <= windows.hi \
         AND events.device = windows.device",
        "many-to-one",
    );
    let dict_path = fixture(&yaml);
    let dict = load(&dict_path);
    assert_validates(&dict_path, &dict);
}

#[test]
fn unique_eq_copy_on_one_to_one_passes_validate_data() {
    // the sharp edge: a *unique* copy column only stays distinct if the
    // copy draws the same owner as the probe (identity on one-to-one) and
    // the source is itself unique. a wrong owner draw would not break D05
    // (zero matches are legal) — it breaks D03, which is what this test
    // pins down
    let yaml = range_dict(
        "      - name: device\n        type: INTEGER\n        constraints: [unique]\n",
        "      - name: device\n        type: INTEGER\n        constraints: [unique]\n",
        "",
        "events.at >= windows.lo AND events.at <= windows.hi \
         AND events.device = windows.device",
        "one-to-one",
    );
    let dict_path = fixture(&yaml);
    let dict = load(&dict_path);
    assert_validates(&dict_path, &dict);
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
fn non_orderable_range_column_is_refused() {
    // MAP values generate fine for unique/fk columns (injective), but nth
    // is not monotone for them — slots would not be ordered intervals
    let dict_path = fixture(indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: windows
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: lo
                type: MAP(VARCHAR, INTEGER)
                constraints: [unique]
              - name: hi
                type: MAP(VARCHAR, INTEGER)
          - name: events
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: at
                type: MAP(VARCHAR, INTEGER)
        relationships:
          - join: events.at >= windows.lo AND events.at <= windows.hi
            cardinality: many-to-one
    "#});
    let dict = load(&dict_path);
    let err = generate(&dict, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(
            err,
            GenerateError::RangeUnsupported {
                ref table,
                ref column,
                ref reason,
            } if table == "windows" && column == "lo" && reason.contains("orderable")
        ),
        "got: {err:?}"
    );
}

#[test]
fn mismatched_range_column_types_are_refused() {
    // DATE probe against TIMESTAMP bounds: each type's nth sequence is
    // monotone on its own, but index arithmetic across two different
    // sequences proves nothing about containment
    let dict_path = fixture(indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: windows
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: lo
                type: TIMESTAMP
                constraints: [unique]
              - name: hi
                type: TIMESTAMP
          - name: events
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: at
                type: DATE
        relationships:
          - join: events.at >= windows.lo AND events.at <= windows.hi
            cardinality: many-to-one
    "#});
    let dict = load(&dict_path);
    let err = generate(&dict, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(
            err,
            GenerateError::RangeUnsupported {
                ref table,
                ref column,
                ..
            } if table == "events" && column == "at"
        ),
        "got: {err:?}"
    );
}

#[test]
fn eq_copy_type_mismatch_with_its_source_is_refused() {
    // the copy renders the source's literal — under a different column
    // type the stored value could silently diverge, so refuse instead
    let dict_path = fixture(indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: windows
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: lo
                type: TIMESTAMP
                constraints: [unique]
              - name: hi
                type: TIMESTAMP
              - name: device
                type: INTEGER
          - name: events
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: at
                type: TIMESTAMP
              - name: device
                type: VARCHAR
        relationships:
          - join: events.at >= windows.lo AND events.at <= windows.hi
              AND events.device = windows.device
            cardinality: many-to-one
    "#});
    let dict = load(&dict_path);
    let err = generate(&dict, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(
            err,
            GenerateError::RangeUnsupported {
                ref table,
                ref column,
                ..
            } if table == "events" && column == "device"
        ),
        "got: {err:?}"
    );
}

#[test]
fn eq_copy_from_an_untyped_source_is_refused() {
    // an untyped column never reaches the DDL or the INSERTs, so there is
    // no stored value to copy — refuse descriptively rather than panic
    let dict_path = fixture(indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: windows
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: lo
                type: TIMESTAMP
                constraints: [unique]
              - name: hi
                type: TIMESTAMP
              - name: device
          - name: events
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: at
                type: TIMESTAMP
              - name: device
                type: INTEGER
        relationships:
          - join: events.at >= windows.lo AND events.at <= windows.hi
              AND events.device = windows.device
            cardinality: many-to-one
    "#});
    let dict = load(&dict_path);
    let err = generate(&dict, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(
            err,
            GenerateError::RangeUnsupported {
                ref table,
                ref column,
                ..
            } if table == "events" && column == "device"
        ),
        "got: {err:?}"
    );
}

#[test]
fn range_slots_exceeding_type_capacity_are_refused() {
    // nth uses TINYINT's non-negative half (128 values); 100 one-side
    // rows need 300 slot values (3 per row) — refused up front,
    // mirroring the unique-capacity check
    let dict_path = fixture(indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: windows
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: lo
                type: TINYINT
                constraints: [unique]
              - name: hi
                type: TINYINT
          - name: events
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: at
                type: TINYINT
        relationships:
          - join: events.at >= windows.lo AND events.at <= windows.hi
            cardinality: many-to-one
    "#});
    let dict = load(&dict_path);
    let opts = GenerateOptions {
        table_rows: std::collections::HashMap::from([("windows".to_string(), 100)]),
        ..GenerateOptions::default()
    };
    let err = generate(&dict, &opts).unwrap_err();
    assert!(
        matches!(
            err,
            GenerateError::RangeCapacityTooSmall {
                ref table,
                ref column,
                capacity: 128,
                rows: 100,
            } if table == "windows" && column == "lo"
        ),
        "got: {err:?}"
    );
}

#[test]
fn eq_copy_from_a_non_recomputable_source_is_refused() {
    // the eq source (windows.code) is a plain foreign key, so its value is a
    // seed-dependent draw the copy cannot reproduce by index alone. refuse
    // up front with an actionable message rather than dying mid-generation
    // with an internal error
    let dict_path = fixture(indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: other
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
          - name: windows
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: lo
                type: INTEGER
                constraints: [unique]
              - name: hi
                type: INTEGER
              - name: code
                type: INTEGER
                constraints: [foreign_key]
          - name: events
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: at
                type: INTEGER
              - name: wcode
                type: INTEGER
        relationships:
          - join: events.at >= windows.lo AND events.at <= windows.hi
              AND events.wcode = windows.code
            cardinality: many-to-one
          - join: windows.code = other.id
            cardinality: many-to-one
    "#});
    let dict = load(&dict_path);
    let err = generate(&dict, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(
            err,
            GenerateError::RangeUnsupported {
                ref table,
                ref column,
                ..
            } if table == "events" && column == "wcode"
        ),
        "got: {err:?}"
    );
}

#[test]
fn untyped_range_bound_column_is_refused() {
    // a slot bound with no declared type would be dropped from the DDL and
    // the inserts, leaving the join to reference a column that does not
    // exist — refuse cleanly instead of emitting a database the oracle
    // cannot check
    let dict_path = fixture(indoc! {r#"
        $version: "0.2.0"
        $learn_more: https://github.com/pjc-wspace/dbdict
        source:
          duckdb:
            file: data.duckdb
        tables:
          - name: windows
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: lo
                constraints: [unique]
              - name: hi
                type: INTEGER
          - name: events
            columns:
              - name: id
                type: INTEGER
                constraints: [primary_key]
              - name: at
                type: INTEGER
        relationships:
          - join: events.at >= windows.lo AND events.at <= windows.hi
            cardinality: many-to-one
    "#});
    let dict = load(&dict_path);
    let err = generate(&dict, &GenerateOptions::default()).unwrap_err();
    assert!(
        matches!(
            err,
            GenerateError::RangeUnsupported {
                ref table,
                ref column,
                ..
            } if table == "windows" && column == "lo"
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
