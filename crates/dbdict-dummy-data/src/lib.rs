//! Backend-generic dummy-data generation for `dbdict.yaml` dictionaries.
//!
//! This crate turns the lowered model (`dbdict::model::DataDict`) plus
//! generation options into a *plan*: which tables to fill in what order,
//! how many rows each gets, and what each column's values must satisfy
//! (unique key, foreign-key draw, plain fill). It deliberately knows
//! nothing about concrete DuckDB types — rendering a plan into typed SQL
//! literals is `dbdict-dummy-data-duckdb`'s job.
//!
//! The plan builder lands in a later phase; for now the crate holds the
//! shared error type so the DuckDB half has something to compose with.

use std::fmt;

/// Why a dictionary cannot be turned into a generation plan.
#[derive(Debug)]
pub enum DummyDataError {
    /// dummy data is a rich-format feature; the legacy (parquet) path is
    /// validation-only
    LegacyUnsupported,
}

impl fmt::Display for DummyDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DummyDataError::LegacyUnsupported => write!(
                f,
                "dummy data can only be generated for rich (0.2.0) dictionaries"
            ),
        }
    }
}

impl std::error::Error for DummyDataError {}
