//! Errors from the DuckDB shell-out reader.

use std::fmt;

#[derive(Debug)]
pub enum DuckdbError {
    /// The `duckdb` CLI was not found on PATH.
    NotFound,
    /// The `duckdb` CLI ran but exited non-zero.
    Cli { status: Option<i32>, stderr: String },
    /// The CLI output could not be parsed.
    Parse(String),
}

impl fmt::Display for DuckdbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DuckdbError::NotFound => write!(
                f,
                "the `duckdb` CLI was not found on PATH (install DuckDB, \
                 or build without the `duckdb` feature)"
            ),
            DuckdbError::Cli { status, stderr } => {
                let code = status.map_or_else(|| "unknown".to_string(), |c| c.to_string());
                write!(f, "duckdb exited with status {code}: {}", stderr.trim())
            }
            DuckdbError::Parse(msg) => write!(f, "could not parse duckdb output: {msg}"),
        }
    }
}

impl std::error::Error for DuckdbError {}
