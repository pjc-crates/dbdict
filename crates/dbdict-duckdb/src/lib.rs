//! DuckDB backend for dbdict.yaml validation.
//!
//! Built on the bundled `duckdb` crate: everything runs in-process, so the
//! binary is self-contained and no `duckdb` CLI is needed on PATH. This crate
//! drives the rich (0.2.0) round-trip — [`read_schema`] reads the real
//! database, [`instantiate`] builds the dictionary's scratch schema,
//! [`classify`] buckets canonical types, [`expand_typedefs`] reports each
//! typedef's canonical expansion, and [`NativeDuckdb`] wires the seam methods
//! into the core `DuckdbBackend` trait.

mod native;

pub use native::{
    NativeDuckdb, TypedefExpansion, classify, count_duplicate_keys, count_nulls, expand_typedefs,
    instantiate, read_schema,
};
