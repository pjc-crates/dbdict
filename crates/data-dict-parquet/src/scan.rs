use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;
use parquet::schema::types::Type;

use crate::ParquetError;

/// What a column's data must be inspected for.
#[derive(Default, Clone)]
pub struct ColumnNeeds {
    /// Count nulls and sample the row numbers where they occur.
    pub nulls: bool,
}

impl ColumnNeeds {
    pub fn any(&self) -> bool {
        self.nulls
    }

    pub fn merge(self, other: Self) -> Self {
        ColumnNeeds {
            nulls: self.nulls || other.nulls,
        }
    }
}

/// Statistics gathered by scanning a column's values.
#[derive(Default)]
pub struct ColumnStats {
    pub null_count: usize,
    /// 1-based row numbers, capped by the caller's limit.
    pub null_rows: Vec<usize>,
}

/// Gather requested statistics in one projected, streaming pass over the file.
pub fn column_stats(
    path: &Path,
    needs: &HashMap<String, ColumnNeeds>,
    limit: usize,
) -> Result<HashMap<String, ColumnStats>, ParquetError> {
    let file =
        File::open(path).map_err(|e| ParquetError::General(format!("Cannot open file: {e}")))?;
    let reader = SerializedFileReader::new(file)?;
    let schema = reader.metadata().file_metadata().schema();

    let requested: Vec<(String, usize, &ColumnNeeds)> = needs
        .iter()
        .filter(|(_, need)| need.any())
        .filter_map(|(name, need)| {
            schema
                .get_fields()
                .iter()
                .position(|field| field.name() == name)
                .map(|index| (name.clone(), index, need))
        })
        .collect();

    let mut stats: HashMap<String, ColumnStats> = requested
        .iter()
        .map(|(name, _, _)| (name.clone(), ColumnStats::default()))
        .collect();
    let to_scan: Vec<usize> = requested
        .iter()
        .filter(|(_, _, need)| need.nulls)
        .map(|(_, index, _)| *index)
        .collect();

    if to_scan.is_empty() {
        return Ok(stats);
    }

    let projection = Type::group_type_builder("schema")
        .with_fields(
            to_scan
                .iter()
                .map(|&index| schema.get_fields()[index].clone())
                .collect(),
        )
        .build()?;

    for (index, row) in reader.get_row_iter(Some(projection))?.enumerate() {
        let row = row?;
        for (name, field) in row.get_column_iter() {
            let (Some(stat), Some(need)) = (stats.get_mut(name), needs.get(name)) else {
                continue;
            };
            if need.nulls && matches!(field, Field::Null) {
                stat.null_count += 1;
                if stat.null_rows.len() < limit {
                    stat.null_rows.push(index + 1);
                }
            }
        }
    }

    Ok(stats)
}
