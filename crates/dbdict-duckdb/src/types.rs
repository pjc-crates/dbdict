//! Map a DuckDB `DESCRIBE` `column_type` string to a coarse dbdict type,
//! matching the vocabulary the parquet reader emits (`number`, `string`,
//! `boolean`, `date`, `datetime`, `enum`).
//!
//! DuckDB reports fully-resolved, parameterised spellings (`DECIMAL(9,2)`,
//! `TIMESTAMP WITH TIME ZONE`, `ENUM('a', 'b')`, `INTEGER[]`), so matching
//! normalises to uppercase and keys off prefixes rather than exact equality.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictType {
    Number,
    String,
    Boolean,
    Date,
    Datetime,
    Enum,
    Unsupported,
}

impl DictType {
    /// The coarse dict-type string, matching the parquet reader's vocabulary.
    pub fn as_str(self) -> &'static str {
        match self {
            DictType::Number => "number",
            DictType::String => "string",
            DictType::Boolean => "boolean",
            DictType::Date => "date",
            DictType::Datetime => "datetime",
            DictType::Enum => "enum",
            DictType::Unsupported => "unsupported",
        }
    }
}

/// Map a DuckDB `column_type` to a coarse [`DictType`].
pub fn dict_type_for(column_type: &str) -> DictType {
    let up = column_type.trim().to_ascii_uppercase();

    // arrays (`INTEGER[]`, `INTEGER[3]`, `VARCHAR[]`) have no dict equivalent
    if up.contains('[') {
        return DictType::Unsupported;
    }

    // prefix families: parameterised or multi-word spellings
    if up.starts_with("TIMESTAMP") {
        return DictType::Datetime;
    }
    if up.starts_with("ENUM") {
        return DictType::Enum;
    }
    if up.starts_with("DECIMAL") || up.starts_with("NUMERIC") {
        return DictType::Number;
    }
    if up.starts_with("STRUCT")
        || up.starts_with("MAP")
        || up.starts_with("LIST")
        || up.starts_with("UNION")
    {
        return DictType::Unsupported;
    }

    // base name, before any parameters or trailing words
    let base = up.split(['(', ' ']).next().unwrap_or("").trim();
    match base {
        "TINYINT" | "SMALLINT" | "INTEGER" | "BIGINT" | "HUGEINT" | "UTINYINT"
        | "USMALLINT" | "UINTEGER" | "UBIGINT" | "UHUGEINT" | "FLOAT" | "REAL"
        | "DOUBLE" | "INT1" | "INT2" | "INT4" | "INT8" | "SHORT" | "INT"
        | "SIGNED" | "LONG" | "FLOAT4" | "FLOAT8" => DictType::Number,

        "VARCHAR" | "CHAR" | "BPCHAR" | "TEXT" | "STRING" | "BLOB" | "BYTEA"
        | "BINARY" | "VARBINARY" | "UUID" | "TIME" => DictType::String,

        "BOOLEAN" | "BOOL" | "LOGICAL" => DictType::Boolean,
        "DATE" => DictType::Date,

        _ => DictType::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_and_reals_are_number() {
        for t in [
            "TINYINT", "SMALLINT", "INTEGER", "BIGINT", "HUGEINT", "UTINYINT",
            "USMALLINT", "UINTEGER", "UBIGINT", "UHUGEINT", "FLOAT", "REAL", "DOUBLE",
        ] {
            assert_eq!(dict_type_for(t), DictType::Number, "{t}");
        }
    }

    #[test]
    fn parameterised_decimal_is_number() {
        assert_eq!(dict_type_for("DECIMAL(9,2)"), DictType::Number);
        assert_eq!(dict_type_for("NUMERIC(10,0)"), DictType::Number);
    }

    #[test]
    fn varchar_family_is_string() {
        for t in ["VARCHAR", "TEXT", "STRING", "CHAR", "BPCHAR"] {
            assert_eq!(dict_type_for(t), DictType::String, "{t}");
        }
    }

    #[test]
    fn boolean_is_boolean() {
        assert_eq!(dict_type_for("BOOLEAN"), DictType::Boolean);
    }

    #[test]
    fn date_is_date() {
        assert_eq!(dict_type_for("DATE"), DictType::Date);
    }

    #[test]
    fn timestamp_family_is_datetime() {
        assert_eq!(dict_type_for("TIMESTAMP"), DictType::Datetime);
        assert_eq!(dict_type_for("TIMESTAMP WITH TIME ZONE"), DictType::Datetime);
        assert_eq!(dict_type_for("TIMESTAMP_MS"), DictType::Datetime);
    }

    #[test]
    fn enum_inline_definition_is_enum() {
        assert_eq!(dict_type_for("ENUM('happy', 'sad')"), DictType::Enum);
    }

    #[test]
    fn edge_scalars_map_to_string() {
        for t in ["TIME", "BLOB", "UUID"] {
            assert_eq!(dict_type_for(t), DictType::String, "{t}");
        }
    }

    #[test]
    fn nested_and_arrays_are_unsupported() {
        for t in [
            "INTEGER[]",
            "VARCHAR[]",
            "INTEGER[3]",
            "STRUCT(a INTEGER)",
            "MAP(VARCHAR, INTEGER)",
            "LIST(INTEGER)",
        ] {
            assert_eq!(dict_type_for(t), DictType::Unsupported, "{t}");
        }
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(dict_type_for("integer"), DictType::Number);
        assert_eq!(dict_type_for("timestamp with time zone"), DictType::Datetime);
    }
}
