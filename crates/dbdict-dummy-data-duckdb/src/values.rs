//! Deterministic indexed value generation: `nth(type, i)` renders the
//! `i`-th value of a type as a SQL literal.
//!
//! Two properties carry the whole generator design (see the session plan):
//!
//! * **injective** — distinct `i` produce distinct values, for every
//!   supported type. Uniqueness (D02/D03) then costs nothing, and a
//!   foreign key can reference the target's `k`-th primary-key value by
//!   just computing `nth(pk_type, k)` — no reading back from the database.
//! * **monotone** (orderable types only, see [`is_orderable`]) — larger
//!   `i` produce strictly larger values, so range-join construction (D05)
//!   reduces to index arithmetic.
//!
//! Values are boring on purpose (`0, 1, 2, …` shaped): the dictionary only
//! promises type- and constraint-correctness, not realism. Randomness for
//! plain fill columns is the *caller's* job — it picks random indices; the
//! mapping from index to value stays pure so the two properties hold.

use crate::types::DuckType;
use std::fmt;

/// Why a value could not be generated.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueError {
    /// the type cannot produce `capacity` distinct values — e.g. a unique
    /// BOOLEAN column asked for a third value, or an ENUM with fewer
    /// variants than requested rows
    Exhausted { ty: String, capacity: u64 },
    /// the type is outside the supported surface (`DuckType::Unsupported`)
    Unsupported { raw: String },
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueError::Exhausted { ty, capacity } => {
                write!(f, "type {ty} can only produce {capacity} distinct value(s)")
            }
            ValueError::Unsupported { raw } => {
                write!(f, "cannot generate values for unsupported type {raw}")
            }
        }
    }
}

impl std::error::Error for ValueError {}

/// How many distinct values [`nth`] can produce for this type before
/// returning [`ValueError::Exhausted`]. Practically-unbounded types report
/// `u64::MAX`. A nested type's capacity is the *minimum* of its parts,
/// because [`nth`] drives every part with the same index.
pub fn capacity(ty: &DuckType) -> u64 {
    match ty {
        DuckType::Boolean => 2,
        // signed types start at 0, so only the non-negative half is used —
        // plenty, and it keeps every literal the plain digits of `i`
        DuckType::TinyInt => 128,
        DuckType::SmallInt => 1 << 15,
        DuckType::Integer => 1 << 31,
        DuckType::BigInt | DuckType::HugeInt => u64::MAX,
        DuckType::UTinyInt => 256,
        DuckType::USmallInt => 1 << 16,
        DuckType::UInteger => 1 << 32,
        DuckType::UBigInt | DuckType::UHugeInt => u64::MAX,
        // exact integer range of the mantissa: beyond it consecutive
        // integers collide, which would break injectivity
        DuckType::Float => 1 << 24,
        DuckType::Double => 1 << 53,
        // integer-part digits; scale digits are always zero in our literals
        DuckType::Decimal { width, scale } => {
            let digits = width.saturating_sub(*scale);
            10u64.checked_pow(digits as u32).unwrap_or(u64::MAX)
        }
        DuckType::Varchar | DuckType::Blob | DuckType::Bit | DuckType::Json => u64::MAX,
        // 2000-01-01 plus up to ~7900 years of days stays within DATE range
        DuckType::Date => 2_890_000,
        // one value per second of the day
        DuckType::Time => 86_400,
        // one value per second from 2000-01-01 to well past year 9999
        DuckType::Timestamp | DuckType::TimestampTz => 250_000_000_000,
        DuckType::Interval => u64::MAX,
        // the 48 bits we format into the last uuid group
        DuckType::Uuid => 1 << 48,
        DuckType::Geometry => u64::MAX,
        DuckType::Enum(values) => values.len() as u64,
        DuckType::List(inner) | DuckType::Array(inner, _) => capacity(inner),
        DuckType::Struct(fields) | DuckType::Union(fields) => {
            fields.iter().map(|(_, t)| capacity(t)).min().unwrap_or(0)
        }
        DuckType::Map(key, value) => capacity(key).min(capacity(value)),
        DuckType::Unsupported(_) => 0,
    }
}

/// Whether [`nth`] is *monotone* for this type: `i < j` implies
/// `nth(i) < nth(j)` under the type's own ordering. Only monotone types
/// can serve as range-join bounds; everything else is injective-only
/// (fine for unique and foreign-key columns).
pub fn is_orderable(ty: &DuckType) -> bool {
    matches!(
        ty,
        DuckType::TinyInt
            | DuckType::SmallInt
            | DuckType::Integer
            | DuckType::BigInt
            | DuckType::HugeInt
            | DuckType::UTinyInt
            | DuckType::USmallInt
            | DuckType::UInteger
            | DuckType::UBigInt
            | DuckType::UHugeInt
            | DuckType::Float
            | DuckType::Double
            | DuckType::Decimal { .. }
            | DuckType::Varchar
            | DuckType::Date
            | DuckType::Time
            | DuckType::Timestamp
            | DuckType::TimestampTz
            | DuckType::Interval
    )
}

/// The `i`-th value of `ty`, rendered as a SQL literal ready to place in
/// an INSERT. Injective in `i`; monotone when [`is_orderable`]. `i` at or
/// beyond [`capacity`] is an error, never a wraparound.
pub fn nth(ty: &DuckType, i: u64) -> Result<String, ValueError> {
    if let DuckType::Unsupported(raw) = ty {
        return Err(ValueError::Unsupported { raw: raw.clone() });
    }
    let cap = capacity(ty);
    if i >= cap {
        return Err(ValueError::Exhausted {
            ty: type_name(ty),
            capacity: cap,
        });
    }
    let literal = match ty {
        DuckType::Boolean => {
            if i == 0 {
                "false".to_string()
            } else {
                "true".to_string()
            }
        }
        DuckType::TinyInt
        | DuckType::SmallInt
        | DuckType::Integer
        | DuckType::BigInt
        | DuckType::HugeInt
        | DuckType::UTinyInt
        | DuckType::USmallInt
        | DuckType::UInteger
        | DuckType::UBigInt
        | DuckType::UHugeInt => i.to_string(),
        // `{i}.0` keeps the literal unambiguously floating-point
        DuckType::Float | DuckType::Double => format!("{i}.0"),
        // whole units: the scale digits stay zero, so the integer literal
        // casts exactly
        DuckType::Decimal { .. } => i.to_string(),
        // zero-padded so string order agrees with numeric order (monotone)
        DuckType::Varchar => format!("'v{i:019}'"),
        // 8 bytes, big-endian, each as a `\xHH` escape
        DuckType::Blob => {
            let mut out = String::from("'");
            for byte in i.to_be_bytes() {
                out.push_str(&format!("\\x{byte:02X}"));
            }
            out.push_str("'::BLOB");
            out
        }
        // a 64-character bitstring of `i` in binary
        DuckType::Bit => format!("'{i:064b}'::BIT"),
        DuckType::Date => {
            let base = chrono::NaiveDate::from_ymd_opt(2000, 1, 1).expect("valid base date");
            let date = base + chrono::Days::new(i);
            format!("DATE '{date}'")
        }
        DuckType::Time => {
            let (h, m, s) = (i / 3600, (i / 60) % 60, i % 60);
            format!("TIME '{h:02}:{m:02}:{s:02}'")
        }
        DuckType::Timestamp => format!("TIMESTAMP '{}'", timestamp_text(i)),
        DuckType::TimestampTz => format!("TIMESTAMPTZ '{}+00'", timestamp_text(i)),
        DuckType::Interval => format!("INTERVAL {i} SECONDS"),
        // fixed prefix + 48 bits of `i` in the node segment
        DuckType::Uuid => format!("'00000000-0000-4000-8000-{i:012x}'::UUID"),
        // a tiny but genuinely-JSON document
        DuckType::Json => format!("'{{\"i\":{i}}}'::JSON"),
        // WKT text cast to the built-in GEOMETRY type; distinct x per `i`
        DuckType::Geometry => format!("'POINT ({i} 0)'::GEOMETRY"),
        DuckType::Enum(values) => quote_string(&values[i as usize]),
        DuckType::List(inner) => format!("[{}]", nth(inner, i)?),
        DuckType::Array(inner, n) => {
            // first element carries the index (injectivity); the rest pad
            // the array to its declared length with the 0th value
            let mut items = vec![nth(inner, i)?];
            for _ in 1..*n {
                items.push(nth(inner, 0)?);
            }
            format!("[{}]", items.join(", "))
        }
        DuckType::Struct(fields) => {
            let mut items = Vec::new();
            for (name, field_ty) in fields {
                items.push(format!("{}: {}", quote_string(name), nth(field_ty, i)?));
            }
            format!("{{{}}}", items.join(", "))
        }
        DuckType::Map(key, value) => {
            format!("MAP {{{}: {}}}", nth(key, i)?, nth(value, i)?)
        }
        // one alternative is enough: tag every generated value with the
        // first member, varying its payload
        DuckType::Union(alternatives) => {
            let (tag, alt_ty) = &alternatives[0];
            format!(
                "union_value({} := {})",
                quote_ident_sql(tag),
                nth(alt_ty, i)?
            )
        }
        DuckType::Unsupported(_) => unreachable!("handled above"),
    };
    Ok(literal)
}

/// A short display name for error messages.
fn type_name(ty: &DuckType) -> String {
    match ty {
        DuckType::Enum(values) => format!("ENUM with {} value(s)", values.len()),
        DuckType::Decimal { width, scale } => format!("DECIMAL({width},{scale})"),
        other => format!("{other:?}"),
    }
}

/// `2000-01-01 00:00:00` plus `i` seconds, in duckdb timestamp text form.
fn timestamp_text(i: u64) -> String {
    let base = chrono::NaiveDate::from_ymd_opt(2000, 1, 1)
        .expect("valid base date")
        .and_hms_opt(0, 0, 0)
        .expect("valid base time");
    let ts = base + chrono::TimeDelta::seconds(i as i64);
    ts.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// A single-quoted SQL string literal, escaping quotes by doubling.
fn quote_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A double-quoted SQL identifier, escaping quotes by doubling — for the
/// tag name in `union_value(tag := …)`.
fn quote_ident_sql(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
