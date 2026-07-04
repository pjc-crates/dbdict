//! Parquet reader for dbdict.yaml validation.

mod metadata;
mod scan;

pub use metadata::{ColumnMeta, ColumnTypeInfo, column_meta, column_type_info, column_types};
pub use parquet::errors::ParquetError;
pub use scan::{ColumnNeeds, ColumnStats, column_stats};
