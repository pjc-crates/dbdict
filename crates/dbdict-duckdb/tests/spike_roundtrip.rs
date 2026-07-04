//! Phase-1 spike: prove mechanism C (round-trip). Does a table typed with
//! `typedef` aliases produce a `DESCRIBE` that BYTE-MATCHES the same table typed
//! with the fully-expanded native types? If yes, the dict-side and real-side are
//! directly comparable and we do zero substitution ourselves.

use duckdb::Connection;

fn describe(conn: &Connection, table: &str) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare(&format!("DESCRIBE \"{table}\""))
        .expect("prepare DESCRIBE");
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .expect("query DESCRIBE");
    rows.map(|r| r.expect("row")).collect()
}

#[test]
fn alias_typed_describe_byte_matches_native_typed_describe() {
    let conn = Connection::open_in_memory().expect("in-memory db");

    // typedefs, including compounding (address -> mystring) and the type zoo
    conn.execute_batch(
        "
        CREATE TYPE mystring AS VARCHAR;
        CREATE TYPE address AS STRUCT(city mystring, postcode INTEGER);
        CREATE TYPE embedding AS FLOAT[768];
        CREATE TYPE tags AS VARCHAR[];
        CREATE TYPE money AS DECIMAL(9,2);
        CREATE TYPE mood AS ENUM('happy', 'sad');
        CREATE TYPE idmap AS MAP(VARCHAR, INTEGER);
        ",
    )
    .expect("create typedefs");

    // dict-side: columns typed with aliases
    conn.execute_batch(
        "CREATE TABLE dict_side (
            home address, name mystring, vec embedding,
            labels tags, price money, feeling mood, lookup idmap
        );",
    )
    .expect("create dict_side");

    // real-side: same columns typed with fully-expanded native types
    conn.execute_batch(
        "CREATE TABLE real_side (
            home STRUCT(city VARCHAR, postcode INTEGER),
            name VARCHAR,
            vec FLOAT[768],
            labels VARCHAR[],
            price DECIMAL(9,2),
            feeling ENUM('happy', 'sad'),
            lookup MAP(VARCHAR, INTEGER)
        );",
    )
    .expect("create real_side");

    let dict = describe(&conn, "dict_side");
    let real = describe(&conn, "real_side");

    // print for the spike record
    for ((n, dt), (_, rt)) in dict.iter().zip(real.iter()) {
        println!("{n}: dict={dt:?} real={rt:?} {}", if dt == rt { "OK" } else { "MISMATCH" });
    }
    assert_eq!(dict, real, "alias-typed DESCRIBE must byte-match native-typed DESCRIBE");
}

#[test]
fn unknown_or_forward_typedef_errors() {
    let conn = Connection::open_in_memory().expect("in-memory db");
    // `b` does not exist yet -> should error, not silently succeed
    let r = conn.execute_batch("CREATE TYPE a AS b;");
    assert!(r.is_err(), "referencing an undefined type should error, got {r:?}");
}
