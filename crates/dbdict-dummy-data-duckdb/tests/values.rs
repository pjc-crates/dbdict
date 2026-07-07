//! Round-trip the value generator through the real bundled engine: for
//! each supported canonical type, INSERT a run of generated literals into
//! a column of that type and let duckdb prove they are type-correct,
//! distinct (injective), and — where claimed — monotone.

use dbdict_dummy_data_duckdb::{DuckType, ValueError, capacity, is_orderable, nth, parse_type};
use duckdb::Connection;

/// Insert `nth(0..n)` into a fresh table of `canonical` type and assert
/// injectivity (count distinct) plus monotonicity (no order inversions)
/// when the type claims it. `n` is clamped to the type's capacity.
fn roundtrip(canonical: &str, n: u64) {
    let ty = parse_type(canonical);
    assert!(
        !matches!(ty, DuckType::Unsupported(_)),
        "{canonical} unexpectedly parsed as Unsupported"
    );
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(&format!("CREATE TABLE t (idx BIGINT, v {canonical})"), [])
        .unwrap();

    let count = n.min(capacity(&ty));
    assert!(count > 0, "{canonical} has zero capacity");
    for i in 0..count {
        let literal = nth(&ty, i).unwrap();
        conn.execute(&format!("INSERT INTO t VALUES ({i}, {literal})"), [])
            .unwrap_or_else(|e| panic!("INSERT of {canonical} value `{literal}` failed: {e}"));
    }

    // the engine, not this crate, is the judge of distinctness
    let distinct: u64 = conn
        .query_row("SELECT count(DISTINCT v) FROM t", [], |r| r.get(0))
        .unwrap_or_else(|e| panic!("DISTINCT over {canonical} failed: {e}"));
    assert_eq!(distinct, count, "{canonical} values are not injective");

    if is_orderable(&ty) {
        // any pair inserted in index order but not in value order is an
        // inversion — zero of them means strictly monotone
        let inversions: u64 = conn
            .query_row(
                "SELECT count(*) FROM t a JOIN t b ON a.idx < b.idx AND a.v >= b.v",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(inversions, 0, "{canonical} values are not monotone");
    }
}

// --- scalars ---------------------------------------------------------------

#[test]
fn booleans() {
    roundtrip("BOOLEAN", 20); // clamps to capacity 2
}

#[test]
fn integer_family() {
    for t in [
        "TINYINT",
        "SMALLINT",
        "INTEGER",
        "BIGINT",
        "HUGEINT",
        "UTINYINT",
        "USMALLINT",
        "UINTEGER",
        "UBIGINT",
        "UHUGEINT",
    ] {
        roundtrip(t, 20);
    }
}

#[test]
fn floats() {
    roundtrip("FLOAT", 20);
    roundtrip("DOUBLE", 20);
}

#[test]
fn decimals() {
    roundtrip("DECIMAL(18,4)", 20);
    roundtrip("DECIMAL(3,2)", 20); // one integer digit: clamps to capacity 10
}

#[test]
fn strings_and_bytes() {
    roundtrip("VARCHAR", 20);
    roundtrip("BLOB", 20);
    roundtrip("BIT", 20);
}

#[test]
fn temporal_types() {
    for t in [
        "DATE",
        "TIME",
        "TIMESTAMP",
        "TIMESTAMP WITH TIME ZONE",
        "INTERVAL",
    ] {
        roundtrip(t, 20);
    }
}

#[test]
fn uuid_values() {
    roundtrip("UUID", 20);
}

// --- extension-backed and 1.5 built-in types --------------------------------
// json is statically linked into our bundled engine; GEOMETRY (plain form)
// is a built-in type since duckdb v1.5 — both confirmed by these tests
// running under the default in-memory config with nothing on disk

#[test]
fn json_values() {
    roundtrip("JSON", 20);
}

#[test]
fn geometry_values() {
    roundtrip("GEOMETRY", 20);
}

// --- nested types ------------------------------------------------------------

#[test]
fn enums() {
    roundtrip("ENUM('buy', 'sell')", 20); // clamps to capacity 2
}

#[test]
fn lists_and_arrays() {
    roundtrip("INTEGER[]", 20);
    roundtrip("INTEGER[3]", 20);
    roundtrip("VARCHAR[][]", 20);
}

#[test]
fn structs() {
    roundtrip("STRUCT(a INTEGER, \"b c\" VARCHAR)", 20);
}

#[test]
fn maps_and_unions() {
    roundtrip("MAP(VARCHAR, INTEGER)", 20);
    roundtrip("UNION(num INTEGER, str VARCHAR)", 20);
}

// struct-in-struct with list members: proves the recursion at the value
// level, not just end-to-end
#[test]
fn deeply_nested_struct() {
    roundtrip(
        "STRUCT(nested STRUCT(x INTEGER, tags VARCHAR[]), xs INTEGER[])",
        20,
    );
}

// --- capacity and refusal -----------------------------------------------------

#[test]
fn exhausted_types_error_instead_of_wrapping() {
    let boolean = parse_type("BOOLEAN");
    assert!(matches!(
        nth(&boolean, 2),
        Err(ValueError::Exhausted { capacity: 2, .. })
    ));

    let enum_ty = parse_type("ENUM('buy', 'sell')");
    assert!(matches!(
        nth(&enum_ty, 2),
        Err(ValueError::Exhausted { capacity: 2, .. })
    ));
}

#[test]
fn nested_capacity_is_the_minimum_of_the_parts() {
    // a struct driven by one index can only produce as many distinct
    // values as its narrowest field
    let ty = parse_type("STRUCT(flag BOOLEAN, n INTEGER)");
    assert_eq!(capacity(&ty), 2);
}

// --- capacity honesty at the top of the index range --------------------------
// plain-fill draws indices uniformly from [0, capacity), so the top of the
// range is the common case, not an edge case: every capacity must be honest
// about what the engine actually accepts (code review 2026-07-08)

/// Insert the single literal `nth(ty, i)` into a fresh column of
/// `canonical` type — the engine judges whether the capacity is honest at
/// its very top index.
fn engine_accepts(canonical: &str, i: u64) {
    let ty = parse_type(canonical);
    let literal = nth(&ty, i).unwrap();
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(&format!("CREATE TABLE t (v {canonical})"), [])
        .unwrap();
    conn.execute(&format!("INSERT INTO t VALUES ({literal})"), [])
        .unwrap_or_else(|e| panic!("INSERT of {canonical} value `{literal}` failed: {e}"));
}

#[test]
fn bigint_capacity_fits_the_signed_range() {
    // BIGINT is a signed i64: only the non-negative half is usable
    assert_eq!(capacity(&parse_type("BIGINT")), 1 << 63);
    engine_accepts("BIGINT", (1 << 63) - 1);
}

#[test]
fn interval_capacity_fits_the_literal_syntax() {
    // the engine rejects `INTERVAL {i} SECONDS` once `i` needs more than
    // 32 signed bits — the literal syntax, not int64 microseconds, binds
    assert_eq!(capacity(&parse_type("INTERVAL")), 1 << 31);
    engine_accepts("INTERVAL", (1 << 31) - 1);
}

#[test]
fn union_capacity_follows_the_first_alternative_only() {
    // nth varies only the first alternative's payload, so a narrow later
    // alternative must not shrink the capacity below what nth can produce
    assert_eq!(
        capacity(&parse_type("UNION(num INTEGER, flag BOOLEAN)")),
        1 << 31
    );
    // the engine confirms 20 distinct values from a union whose narrowest
    // alternative holds only 2
    roundtrip("UNION(num INTEGER, flag BOOLEAN)", 20);
}

#[test]
fn varchar_stays_monotone_past_nineteen_digits() {
    // u64 indices reach 20 digits; the zero-pad must cover all of them or
    // lexicographic order inverts at the 19→20 digit boundary
    let ty = parse_type("VARCHAR");
    let below = nth(&ty, 9_999_999_999_999_999_999).unwrap();
    let above = nth(&ty, 10_000_000_000_000_000_000).unwrap();
    assert!(
        below < above,
        "monotone contract violated: nth(1e19-1) = {below} does not sort before nth(1e19) = {above}"
    );
}

// INET cannot be statically linked by our duckdb crate version, and the
// CRS-parameterized GEOMETRY needs the spatial extension's coordinate
// registry: both are refused with a descriptive error, never generated
#[test]
fn unsupported_types_are_refused_descriptively() {
    for raw in ["INET", "GEOMETRY('EPSG:2193')"] {
        let ty = parse_type(raw);
        assert!(
            matches!(ty, DuckType::Unsupported(_)),
            "{raw} should be Unsupported"
        );
        let err = nth(&ty, 0).unwrap_err();
        assert!(
            err.to_string().contains(raw),
            "error should name the type: {err}"
        );
    }
}
