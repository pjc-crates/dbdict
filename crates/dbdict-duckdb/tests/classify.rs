//! The classifier is fed canonical spellings straight out of a real
//! `DESCRIBE`, so these expectations can't drift from what duckdb actually
//! produces for the type zoo.

use dbdict::rich::TypeCategory;
use dbdict_duckdb::classify;
use duckdb::Connection;

#[test]
fn classifies_canonical_spellings_from_describe() {
    let conn = Connection::open_in_memory().expect("in-memory db");
    conn.execute_batch(
        "CREATE TABLE zoo (
            a BOOLEAN,
            b BIGINT,
            c DECIMAL(12,2),
            d FLOAT,
            e UHUGEINT,
            f DATE,
            g TIMESTAMP,
            h TIMESTAMP_NS,
            i TIMESTAMP WITH TIME ZONE,
            j ENUM('happy', 'sad'),
            k VARCHAR,
            l STRUCT(city VARCHAR),
            m FLOAT[768],
            n VARCHAR[],
            o MAP(VARCHAR, INTEGER),
            p TIME,
            q UUID,
            r BLOB,
            s INTERVAL,
            t DECIMAL(12,2)[],
            u ENUM('a', 'b')[3]
        );",
    )
    .expect("create zoo");

    let mut stmt = conn.prepare("DESCRIBE zoo").expect("prepare");
    let canonical: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("describe")
        .map(|r| r.expect("row"))
        .collect();

    use TypeCategory::*;
    let expected = [
        Boolean,     // BOOLEAN
        Numeric,     // BIGINT
        Numeric,     // DECIMAL(12,2)
        Numeric,     // FLOAT
        Numeric,     // UHUGEINT
        Date,        // DATE
        Timestamp,   // TIMESTAMP
        Timestamp,   // TIMESTAMP_NS
        TimestampTz, // TIMESTAMP WITH TIME ZONE
        Enum,        // ENUM('happy', 'sad')
        Other,       // VARCHAR
        Other,       // STRUCT(...)
        Other,       // FLOAT[768]
        Other,       // VARCHAR[]
        Other,       // MAP(...)
        Other,       // TIME
        Other,       // UUID
        Other,       // BLOB
        Other,       // INTERVAL
        Other,       // DECIMAL(12,2)[] — an array of decimals is not numeric
        Other,       // ENUM('a', 'b')[3] — nor is a fixed array of enums an enum
    ];
    assert_eq!(canonical.len(), expected.len());
    for (spelling, want) in canonical.iter().zip(expected) {
        assert_eq!(
            classify(spelling),
            want,
            "canonical spelling {spelling:?} misclassified"
        );
    }
}
