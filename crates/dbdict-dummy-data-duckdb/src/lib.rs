//! DuckDB-specific half of the dummy-data generator.
//!
//! Maps canonical DuckDB column types (as `DESCRIBE` spells them, obtained
//! via `dbdict_duckdb::instantiate`) 1:1 to deterministic value generators.
//! The core trick: `nth(i)` is injective in `i` for every supported type and
//! monotone for orderable ones, so uniqueness and range-join construction
//! reduce to index arithmetic upstream in `dbdict-dummy-data`.

pub mod types;
pub mod values;

pub use types::{DuckType, parse_type};
pub use values::{ValueError, capacity, is_orderable, nth};
