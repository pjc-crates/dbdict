//! Parse canonical DuckDB type spellings into a typed tree.
//!
//! The core model carries a column's type only as a raw string; the
//! *canonical* spelling (what `DESCRIBE` returns, via
//! `dbdict_duckdb::instantiate`) is this parser's input. The grammar is
//! deliberately small because canonical spellings are machine-generated —
//! e.g. `ENUM('buy', 'sell')` always has the space after the comma. The
//! spellings below were pinned empirically against the bundled engine
//! (duckdb 1.5.4).
//!
//! Anything unrecognized parses as [`DuckType::Unsupported`] — never an
//! error here. Total-over-behavior: the *value generator* refuses
//! unsupported types with a descriptive message, so a dictionary using an
//! exotic type gets a clear "cannot generate" rather than a panic or a
//! silently wrong value.

/// A canonical DuckDB column type, 1:1 with the engine's own type system.
#[derive(Debug, Clone, PartialEq)]
pub enum DuckType {
    Boolean,
    TinyInt,
    SmallInt,
    Integer,
    BigInt,
    HugeInt,
    UTinyInt,
    USmallInt,
    UInteger,
    UBigInt,
    UHugeInt,
    Float,
    Double,
    /// `DECIMAL(width, scale)`
    Decimal {
        width: u8,
        scale: u8,
    },
    Varchar,
    Blob,
    Bit,
    Date,
    Time,
    Timestamp,
    /// `TIMESTAMP WITH TIME ZONE` (declared as `TIMESTAMPTZ`)
    TimestampTz,
    Interval,
    Uuid,
    /// json extension type — statically linked into our bundled engine
    Json,
    /// built-in since duckdb v1.5. only the plain form: the
    /// CRS-parameterized `GEOMETRY('EPSG:…')` needs the spatial
    /// extension's coordinate-system registry, which this build lacks,
    /// so it parses as `Unsupported`
    Geometry,
    /// `ENUM('a', 'b')` — the declared values, in declaration order
    Enum(Vec<String>),
    /// `T[]`
    List(Box<DuckType>),
    /// `T[n]` — fixed-length array
    Array(Box<DuckType>, u64),
    /// `STRUCT(name T, …)` — fields in declaration order
    Struct(Vec<(String, DuckType)>),
    /// `MAP(K, V)`
    Map(Box<DuckType>, Box<DuckType>),
    /// `UNION(tag T, …)` — alternatives in declaration order
    Union(Vec<(String, DuckType)>),
    /// anything this module does not recognize (e.g. `INET`, which cannot
    /// be statically linked, or a future engine's new type). carries the
    /// raw spelling for error messages
    Unsupported(String),
}

/// Parse a canonical type spelling. Never fails: unrecognized input
/// becomes [`DuckType::Unsupported`].
pub fn parse_type(canonical: &str) -> DuckType {
    let s = canonical.trim();

    // list/array suffixes bind last: `INTEGER[3][]` is a list of arrays,
    // so the *outermost* constructor is the trailing suffix
    if let Some(open) = array_suffix_start(s) {
        let inner = parse_type(&s[..open]);
        let digits = &s[open + 1..s.len() - 1];
        if digits.is_empty() {
            return DuckType::List(Box::new(inner));
        }
        // the digits are machine-written, so this parse only fails if the
        // spelling isn't canonical after all — Unsupported, not a panic
        if let Ok(n) = digits.parse::<u64>() {
            return DuckType::Array(Box::new(inner), n);
        }
        return DuckType::Unsupported(s.to_string());
    }

    // simple keyword types
    match s {
        "BOOLEAN" => return DuckType::Boolean,
        "TINYINT" => return DuckType::TinyInt,
        "SMALLINT" => return DuckType::SmallInt,
        "INTEGER" => return DuckType::Integer,
        "BIGINT" => return DuckType::BigInt,
        "HUGEINT" => return DuckType::HugeInt,
        "UTINYINT" => return DuckType::UTinyInt,
        "USMALLINT" => return DuckType::USmallInt,
        "UINTEGER" => return DuckType::UInteger,
        "UBIGINT" => return DuckType::UBigInt,
        "UHUGEINT" => return DuckType::UHugeInt,
        "FLOAT" => return DuckType::Float,
        "DOUBLE" => return DuckType::Double,
        "VARCHAR" => return DuckType::Varchar,
        "BLOB" => return DuckType::Blob,
        "BIT" => return DuckType::Bit,
        "DATE" => return DuckType::Date,
        "TIME" => return DuckType::Time,
        "TIMESTAMP" => return DuckType::Timestamp,
        "TIMESTAMP WITH TIME ZONE" => return DuckType::TimestampTz,
        "INTERVAL" => return DuckType::Interval,
        "UUID" => return DuckType::Uuid,
        "JSON" => return DuckType::Json,
        "GEOMETRY" => return DuckType::Geometry,
        _ => {}
    }

    // parameterized types: `NAME( … )` with the closing paren at the end
    if let Some(args) = parenthesized(s, "DECIMAL(") {
        // canonical form is `DECIMAL(width,scale)` — no space
        if let Some((w, sc)) = args.split_once(',')
            && let (Ok(width), Ok(scale)) = (w.trim().parse(), sc.trim().parse())
        {
            return DuckType::Decimal { width, scale };
        }
    } else if let Some(args) = parenthesized(s, "ENUM(") {
        if let Some(values) = parse_enum_values(args) {
            return DuckType::Enum(values);
        }
    } else if let Some(args) = parenthesized(s, "STRUCT(") {
        if let Some(fields) = parse_fields(args) {
            return DuckType::Struct(fields);
        }
    } else if let Some(args) = parenthesized(s, "MAP(") {
        let parts = split_top_level(args);
        if parts.len() == 2 {
            return DuckType::Map(
                Box::new(parse_type(parts[0])),
                Box::new(parse_type(parts[1])),
            );
        }
    } else if let Some(args) = parenthesized(s, "UNION(")
        && let Some(alternatives) = parse_fields(args)
    {
        return DuckType::Union(alternatives);
    }

    DuckType::Unsupported(s.to_string())
}

/// Where a trailing `[…]` suffix starts, if the spelling ends with one:
/// the index of the `[` whose bracket content is empty or all digits.
fn array_suffix_start(s: &str) -> Option<usize> {
    if !s.ends_with(']') {
        return None;
    }
    let open = s.rfind('[')?;
    let digits = &s[open + 1..s.len() - 1];
    if open > 0 && digits.chars().all(|c| c.is_ascii_digit()) {
        Some(open)
    } else {
        None
    }
}

/// The `…` of `PREFIX…)` when `s` starts with `prefix` and ends with `)`.
fn parenthesized<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(prefix)?;
    rest.strip_suffix(')')
}

/// Split on the commas at nesting depth zero, respecting parens/brackets
/// and both quote styles, trimming each piece. `ENUM('a, b', 'c')` splits
/// into two, not three.
fn split_top_level(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut start = 0;
    for (pos, c) in s.char_indices() {
        match c {
            // canonical spellings escape a quote by doubling it (`''`);
            // the doubled pair just toggles the flag twice, landing back
            // in-quote, so no special case is needed
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '(' | '[' if !in_single && !in_double => depth += 1,
            ')' | ']' if !in_single && !in_double => depth = depth.saturating_sub(1),
            ',' if depth == 0 && !in_single && !in_double => {
                parts.push(s[start..pos].trim());
                start = pos + 1;
            }
            _ => {}
        }
    }
    parts.push(s[start..].trim());
    parts
}

/// The values of an `ENUM('a', 'b')` argument list: each item a
/// single-quoted string with `''` as the escape for a literal quote.
fn parse_enum_values(args: &str) -> Option<Vec<String>> {
    let mut values = Vec::new();
    for part in split_top_level(args) {
        let inner = part.strip_prefix('\'')?.strip_suffix('\'')?;
        values.push(inner.replace("''", "'"));
    }
    Some(values)
}

/// The `name TYPE` pairs of a STRUCT/UNION argument list. A name is a bare
/// identifier or a double-quoted one (`""` escaping a literal quote).
fn parse_fields(args: &str) -> Option<Vec<(String, DuckType)>> {
    let mut fields = Vec::new();
    for part in split_top_level(args) {
        let (name, type_text) = if let Some(rest) = part.strip_prefix('"') {
            // find the closing quote, skipping doubled ("") escapes
            let mut end = None;
            let mut chars = rest.char_indices().peekable();
            while let Some((pos, c)) = chars.next() {
                if c == '"' {
                    if matches!(chars.peek(), Some((_, '"'))) {
                        chars.next(); // consume the second half of the escape
                    } else {
                        end = Some(pos);
                        break;
                    }
                }
            }
            let end = end?;
            (rest[..end].replace("\"\"", "\""), rest[end + 1..].trim())
        } else {
            let (name, type_text) = part.split_once(' ')?;
            (name.to_string(), type_text.trim())
        };
        fields.push((name, parse_type(type_text)));
    }
    Some(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_parameterized_scalars() {
        assert_eq!(
            parse_type("DECIMAL(18,4)"),
            DuckType::Decimal {
                width: 18,
                scale: 4
            }
        );
        assert_eq!(
            parse_type("TIMESTAMP WITH TIME ZONE"),
            DuckType::TimestampTz
        );
    }

    #[test]
    fn parses_enum_values_with_escapes() {
        assert_eq!(
            parse_type("ENUM('buy', 'sell', 'it''s')"),
            DuckType::Enum(vec![
                "buy".to_string(),
                "sell".to_string(),
                "it's".to_string()
            ])
        );
    }

    #[test]
    fn suffixes_bind_outermost() {
        // a list of fixed-length arrays: the trailing [] is the outer type
        assert_eq!(
            parse_type("INTEGER[3][]"),
            DuckType::List(Box::new(DuckType::Array(Box::new(DuckType::Integer), 3)))
        );
    }

    #[test]
    fn parses_struct_fields_with_quoted_names() {
        assert_eq!(
            parse_type("STRUCT(a INTEGER, \"b c\" VARCHAR)"),
            DuckType::Struct(vec![
                ("a".to_string(), DuckType::Integer),
                ("b c".to_string(), DuckType::Varchar),
            ])
        );
    }

    #[test]
    fn parses_nested_containers() {
        assert_eq!(
            parse_type("MAP(VARCHAR, STRUCT(x INTEGER[]))"),
            DuckType::Map(
                Box::new(DuckType::Varchar),
                Box::new(DuckType::Struct(vec![(
                    "x".to_string(),
                    DuckType::List(Box::new(DuckType::Integer))
                )]))
            )
        );
    }

    #[test]
    fn a_comma_inside_an_enum_value_does_not_split() {
        assert_eq!(
            parse_type("ENUM('a, b', 'c')"),
            DuckType::Enum(vec!["a, b".to_string(), "c".to_string()])
        );
    }

    #[test]
    fn unknown_spellings_are_unsupported_with_the_raw_text() {
        assert_eq!(
            parse_type("INET"),
            DuckType::Unsupported("INET".to_string())
        );
        assert_eq!(
            parse_type("GEOMETRY('EPSG:2193')"),
            DuckType::Unsupported("GEOMETRY('EPSG:2193')".to_string())
        );
    }
}
