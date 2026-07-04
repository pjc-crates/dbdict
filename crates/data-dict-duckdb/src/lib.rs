//! DuckDB reader for data-dict.yaml validation.
//!
//! Shells out to the `duckdb` CLI (must be on PATH) rather than linking a
//! native library — see the design spec. Only column names and types are read
//! (meta-level validation).

mod error;
mod types;

use std::path::Path;
use std::process::Command;

pub use error::DuckdbError;
pub use types::{DictType, dict_type_for};

/// Column type info for one column, as read from `DESCRIBE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnTypeInfo {
    pub name: String,
    pub dict_type: DictType,
    /// The DuckDB `column_type` spelling, e.g. `DECIMAL(9,2)`.
    pub duckdb_type: String,
}

/// Read column type info for `table` in the DuckDB database at `file`.
///
/// Runs `duckdb -readonly -json <file> -c 'DESCRIBE "<table>";'` — `-readonly`
/// so the user's database is never locked or mutated.
pub fn describe(file: &Path, table: &str) -> Result<Vec<ColumnTypeInfo>, DuckdbError> {
    let sql = format!("DESCRIBE {};", quote_ident(table));
    let output = Command::new("duckdb")
        .arg("-readonly")
        .arg("-json")
        .arg(file)
        .arg("-c")
        .arg(&sql)
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => DuckdbError::NotFound,
            _ => DuckdbError::Cli {
                status: None,
                stderr: e.to_string(),
            },
        })?;

    if !output.status.success() {
        return Err(DuckdbError::Cli {
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    parse_describe(&String::from_utf8_lossy(&output.stdout))
}

/// `(column_name, dict_type_string)` pairs — the shape the validation seam wants.
pub fn column_types(file: &Path, table: &str) -> Result<Vec<(String, String)>, DuckdbError> {
    Ok(describe(file, table)?
        .into_iter()
        .map(|c| (c.name, c.dict_type.as_str().to_string()))
        .collect())
}

/// Double-quote a DuckDB identifier, escaping embedded quotes.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Parse the JSON array produced by `duckdb -json -c 'DESCRIBE …'`.
fn parse_describe(json: &str) -> Result<Vec<ColumnTypeInfo>, DuckdbError> {
    let trimmed = json.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let value: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| DuckdbError::Parse(e.to_string()))?;
    let rows = value
        .as_array()
        .ok_or_else(|| DuckdbError::Parse("expected a JSON array".to_string()))?;
    rows.iter()
        .map(|row| {
            let name = row
                .get("column_name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| DuckdbError::Parse("row missing string `column_name`".to_string()))?;
            let duckdb_type = row
                .get("column_type")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| DuckdbError::Parse("row missing string `column_type`".to_string()))?;
            Ok(ColumnTypeInfo {
                name: name.to_string(),
                dict_type: dict_type_for(duckdb_type),
                duckdb_type: duckdb_type.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_describe_json() {
        // captured verbatim from `duckdb -json -c 'DESCRIBE "t";'` (v1.5.4)
        let json = r#"[{"column_name":"a","column_type":"BIGINT","null":"YES","key":null,"default":null,"extra":null},
{"column_name":"g","column_type":"VARCHAR","null":"YES","key":null,"default":null,"extra":null},
{"column_name":"l","column_type":"TIMESTAMP WITH TIME ZONE","null":"YES","key":null,"default":null,"extra":null}]"#;
        let cols = parse_describe(json).unwrap();
        assert_eq!(
            cols,
            vec![
                ColumnTypeInfo {
                    name: "a".into(),
                    dict_type: DictType::Number,
                    duckdb_type: "BIGINT".into()
                },
                ColumnTypeInfo {
                    name: "g".into(),
                    dict_type: DictType::String,
                    duckdb_type: "VARCHAR".into()
                },
                ColumnTypeInfo {
                    name: "l".into(),
                    dict_type: DictType::Datetime,
                    duckdb_type: "TIMESTAMP WITH TIME ZONE".into()
                },
            ]
        );
    }

    #[test]
    fn empty_output_parses_to_no_columns() {
        assert_eq!(parse_describe("").unwrap(), vec![]);
        assert_eq!(parse_describe("   \n").unwrap(), vec![]);
    }

    #[test]
    fn non_array_json_is_a_parse_error() {
        assert!(matches!(
            parse_describe(r#"{"oops":1}"#),
            Err(DuckdbError::Parse(_))
        ));
    }

    #[test]
    fn quotes_and_escapes_identifiers() {
        assert_eq!(quote_ident("food"), "\"food\"");
        // an embedded double-quote is doubled
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
    }
}
