//! Lower a `quarto_yaml` AST into the typed [`DataDict`] model.
//!
//! Invariant: lowering only runs after the schema has accepted the document,
//! so we may assume the shape conforms (required keys present, enums valid,
//! arrays where arrays are expected). Unexpected shapes are silently dropped
//! rather than panicking — they should be unreachable.

use quarto_source_map::SourceInfo;
use quarto_yaml::YamlWithSourceInfo;

use crate::join_expr::JoinExpr;
use crate::model::{
    Cardinality, Column, Constraint, DataDict, DictSource, Format, Relationship, Representation,
    Scalar, Source, Spanned, Table, Typedef,
};
use crate::problem::{Problem, ProblemSet, Severity};

/// Lower an AST, collecting any lowering problems (currently only S04
/// for unparseable join expressions).
pub fn lower(root: &YamlWithSourceInfo, problems: &mut ProblemSet) -> DataDict {
    // the schema has already pinned `$version` to one of the two supported
    // values, so anything but 0.2.0 means legacy here
    let format = match root
        .get_hash_value("$version")
        .and_then(|v| v.yaml.as_str())
    {
        Some("0.2.0") => Format::Rich,
        _ => Format::Legacy,
    };

    // global typedef aliases (rich format; the schema keeps the key out of
    // legacy documents, so this is simply empty there)
    let typedefs = match root.get_hash_value("typedef") {
        Some(node) => lower_typedefs(node, problems),
        None => Vec::new(),
    };

    // dictionary-level source (rich format): the one duckdb database
    let source = root.get_hash_value("source").and_then(|n| {
        let duckdb = n.get_hash_value("duckdb")?;
        let file = duckdb.get_hash_value("file")?;
        let path = file.yaml.as_str()?;
        Some(DictSource {
            span: n.source_info.clone(),
            file: Spanned::new(path.to_string(), file.source_info.clone()),
        })
    });

    // declared duckdb extensions (rich format): `duckdb: extensions: [...]`.
    // a null item (the parser collapses an empty `- ""` to null) is kept as
    // an empty string so S19 can point at it with a clear message
    let mut extensions = Vec::new();
    if let Some(e_node) = root
        .get_hash_value("duckdb")
        .and_then(|n| n.get_hash_value("extensions"))
        && let Some(items) = e_node.as_array()
    {
        for item in items {
            let name = item.yaml.as_str().unwrap_or("");
            extensions.push(Spanned::new(name.to_string(), item.source_info.clone()));
        }
    }

    let mut tables = Vec::new();
    if let Some(t_node) = root.get_hash_value("tables")
        && let Some(items) = t_node.as_array()
    {
        for item in items {
            if let Some(table) = lower_table(item, problems) {
                tables.push(table);
            }
        }
    }

    let mut relationships = Vec::new();
    if let Some(r_node) = root.get_hash_value("relationships")
        && let Some(items) = r_node.as_array()
    {
        for item in items {
            relationships.push(lower_relationship(item, problems));
        }
    }

    DataDict {
        format,
        typedefs,
        source,
        extensions,
        tables,
        relationships,
    }
}

/// Lower a `typedef:` mapping into its alias pairs, in document order.
/// A non-string *name* is reported (S18) — the schema constrains typedef
/// values only, so a bare `123:` or `true:` key would otherwise vanish
/// silently. A non-string *value* is dropped without a report here because
/// the schema has already rejected the document.
fn lower_typedefs(node: &YamlWithSourceInfo, problems: &mut ProblemSet) -> Vec<Typedef> {
    let Some(entries) = node.as_hash() else {
        return Vec::new();
    };
    let mut typedefs = Vec::new();
    for entry in entries {
        let Some(name) = entry.key.yaml.as_str() else {
            problems.push_spec_error(
                "S18",
                "A typedef name must be a string.",
                "is not a string",
                [entry.key_span.clone()],
            );
            continue;
        };
        let Some(expr) = entry.value.yaml.as_str() else {
            continue;
        };
        typedefs.push(Typedef {
            name: Spanned::new(name.to_string(), entry.key_span.clone()),
            expr: Spanned::new(expr.to_string(), entry.value_span.clone()),
        });
    }
    typedefs
}

fn lower_table(node: &YamlWithSourceInfo, problems: &mut ProblemSet) -> Option<Table> {
    let entries = node.as_hash()?;
    let name_entry = entries
        .iter()
        .find(|e| e.key.yaml.as_str() == Some("name"))?;
    // An empty/null name is kept (as "") so S11 can report it; the parser
    // collapses an empty name to null.
    let name = name_entry.value.yaml.as_str().unwrap_or("");

    let mut columns = Vec::new();
    if let Some(c_node) = node.get_hash_value("columns")
        && let Some(items) = c_node.as_array()
    {
        for col in items {
            if let Some(c) = lower_column(col) {
                columns.push(c);
            }
        }
    }
    let source = node.get_hash_value("source").and_then(|n| {
        let parquet = n.get_hash_value("parquet")?;
        let path = parquet.yaml.as_str()?;
        Some(Source {
            span: n.source_info.clone(),
            parquet: Spanned::new(path.to_string(), parquet.source_info.clone()),
        })
    });
    let key_span = |key: &str| {
        entries
            .iter()
            .find(|e| e.key.yaml.as_str() == Some(key))
            .map(|e| e.key_span.clone())
    };
    Some(Table {
        name: Spanned::new(name.to_string(), name_entry.value_span.clone()),
        label: lower_string_value(node, "label"),
        typedefs: match node.get_hash_value("typedef") {
            Some(td) => lower_typedefs(td, problems),
            None => Vec::new(),
        },
        columns,
        source,
        description: key_span("description"),
        details: key_span("details"),
    })
}

/// Lower a string-valued key of `node` into a `Spanned<String>`, or `None`
/// when the key is absent or not a string.
fn lower_string_value(node: &YamlWithSourceInfo, key: &str) -> Option<Spanned<String>> {
    let entries = node.as_hash()?;
    let entry = entries.iter().find(|e| e.key.yaml.as_str() == Some(key))?;
    let s = entry.value.yaml.as_str()?;
    // value_span, not value.source_info, to match the rest of the file
    // (identical for plain scalars, but one convention beats two)
    Some(Spanned::new(s.to_string(), entry.value_span.clone()))
}

fn lower_column(node: &YamlWithSourceInfo) -> Option<Column> {
    let entries = node.as_hash()?;
    let mut name: Option<Spanned<String>> = None;
    let mut label: Option<Spanned<String>> = None;
    let mut constraints: Vec<Spanned<Constraint>> = Vec::new();
    let mut col_type: Option<Spanned<String>> = None;
    let mut values: Option<SourceInfo> = None;
    let mut range: Option<Representation> = None;
    let mut examples: Option<Representation> = None;
    let mut units: Option<Spanned<String>> = None;
    let mut time_zone: Option<Spanned<String>> = None;
    for entry in entries {
        let Some(key) = entry.key.yaml.as_str() else {
            continue;
        };
        match key {
            "name" => {
                // An empty/null name is kept (as "") so S11 can report it; the
                // parser collapses an empty name to null.
                let s = entry.value.yaml.as_str().unwrap_or("");
                name = Some(Spanned::new(s.to_string(), entry.value_span.clone()));
            }
            "label" => {
                if let Some(s) = entry.value.yaml.as_str() {
                    label = Some(Spanned::new(s.to_string(), entry.value_span.clone()));
                }
            }
            "type" => {
                if let Some(s) = entry.value.yaml.as_str() {
                    col_type = Some(Spanned::new(s.to_string(), entry.value_span.clone()));
                }
            }
            "values" => values = Some(entry.value_span.clone()),
            "range" => {
                range = Some(Representation {
                    span: entry.value_span.clone(),
                    items: lower_scalars(&entry.value),
                });
            }
            "examples" => {
                examples = Some(Representation {
                    span: entry.value_span.clone(),
                    items: lower_scalars(&entry.value),
                });
            }
            "units" => {
                if let Some(s) = entry.value.yaml.as_str() {
                    units = Some(Spanned::new(s.to_string(), entry.value_span.clone()));
                }
            }
            "time_zone" => {
                if let Some(s) = entry.value.yaml.as_str() {
                    time_zone = Some(Spanned::new(s.to_string(), entry.value_span.clone()));
                }
            }
            "constraints" => {
                if let Some(items) = entry.value.as_array() {
                    for c in items {
                        if let Some(s) = c.yaml.as_str()
                            && let Some(parsed) = Constraint::parse(s)
                        {
                            constraints.push(Spanned::new(parsed, c.source_info.clone()));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(Column {
        name: name?,
        label,
        constraints,
        col_type,
        values,
        range,
        examples,
        units,
        time_zone,
    })
}

/// Lower a `range` or `examples` node into its scalar elements with spans.
/// Non-array nodes yield an empty vector (the schema rejects them upstream).
fn lower_scalars(node: &YamlWithSourceInfo) -> Vec<Spanned<Scalar>> {
    let Some(items) = node.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| Spanned::new(lower_scalar(item), item.source_info.clone()))
        .collect()
}

fn lower_scalar(node: &YamlWithSourceInfo) -> Scalar {
    let yaml = &node.yaml;
    if let Some(b) = yaml.as_bool() {
        Scalar::Bool(b)
    } else if let Some(i) = yaml.as_i64() {
        Scalar::Number(i as f64)
    } else if let Some(f) = yaml.as_f64() {
        Scalar::Number(f)
    } else if let Some(s) = yaml.as_str() {
        Scalar::String(s.to_string())
    } else if node.as_array().is_some() || node.as_hash().is_some() {
        Scalar::Compound
    } else {
        Scalar::Null
    }
}

fn lower_relationship(node: &YamlWithSourceInfo, problems: &mut ProblemSet) -> Relationship {
    let entries = node.as_hash().expect("schema guarantees mapping");
    let mut cardinality: Option<Spanned<Cardinality>> = None;
    let mut join_text: Option<Spanned<String>> = None;
    let mut conflicts: Vec<Spanned<String>> = Vec::new();

    for entry in entries {
        let Some(key) = entry.key.yaml.as_str() else {
            continue;
        };
        match key {
            "cardinality" => {
                if let Some(s) = entry.value.yaml.as_str()
                    && let Some(c) = Cardinality::parse(s)
                {
                    cardinality = Some(Spanned::new(c, entry.value_span.clone()));
                }
            }
            "join" => {
                if let Some(s) = entry.value.yaml.as_str() {
                    join_text = Some(Spanned::new(s.to_string(), entry.value_span.clone()));
                }
            }
            "conflicts" => {
                if let Some(items) = entry.value.as_array() {
                    for c in items {
                        if let Some(s) = c.yaml.as_str() {
                            conflicts.push(Spanned::new(s.to_string(), c.source_info.clone()));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let cardinality = cardinality.expect("schema guarantees cardinality is a valid enum");
    let join_text = join_text.expect("schema guarantees join is present and a string");

    let join = match JoinExpr::parse(&join_text.value) {
        Ok(expr) => Some(expr),
        Err(err) => {
            let span =
                crate::problem::subspan(&join_text.span, err.at, err.at.min(join_text.value.len()))
                    .unwrap_or_else(|| join_text.span.clone());
            problems.push(Problem::spec(
                "S04",
                Severity::Error,
                format!("`join` expression does not parse: {}", err.message),
                span,
            ));
            None
        }
    };

    Relationship {
        cardinality,
        join_text,
        join,
        conflicts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceContext;
    use indoc::indoc;

    /// parse `yaml` and lower it, discarding problems — these tests assert
    /// the lowered model's shape, not diagnostics
    fn lower_str(yaml: &str) -> DataDict {
        let doc = quarto_yaml::parse(yaml).expect("test yaml must parse");
        let mut problems = ProblemSet::new(SourceContext::new());
        lower(&doc, &mut problems)
    }

    #[test]
    fn lowers_rich_typedefs_source_and_labels() {
        let dict = lower_str(indoc! {r#"
            $version: "0.2.0"
            typedef:
              money: DECIMAL(18, 4)
              address: STRUCT(city VARCHAR, postcode INTEGER)
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                label: Trade executions
                typedef:
                  money: DECIMAL(12, 2)
                columns:
                  - name: qty
                    label: Quantity
                    type: money
        "#});

        assert_eq!(dict.format, Format::Rich);

        // global typedefs, in document order
        assert_eq!(dict.typedefs.len(), 2);
        assert_eq!(dict.typedefs[0].name.value, "money");
        assert_eq!(dict.typedefs[0].expr.value, "DECIMAL(18, 4)");
        assert_eq!(dict.typedefs[1].name.value, "address");

        // top-level duckdb source
        let source = dict.source.as_ref().expect("dict-level source");
        assert_eq!(source.file.value, "warehouse.duckdb");

        // table: label + scoped typedef
        let table = &dict.tables[0];
        assert_eq!(
            table.label.as_ref().expect("table label").value,
            "Trade executions"
        );
        assert_eq!(table.typedefs.len(), 1);
        assert_eq!(table.typedefs[0].expr.value, "DECIMAL(12, 2)");

        // column: label, type kept verbatim (alias resolution is not lowering's job)
        let col = &table.columns[0];
        assert_eq!(col.label.as_ref().expect("column label").value, "Quantity");
        assert_eq!(col.col_type.as_ref().expect("column type").value, "money");
    }

    #[test]
    fn lowers_declared_duckdb_extensions() {
        let dict = lower_str(indoc! {r#"
            $version: "0.2.0"
            duckdb:
              extensions:
                - json
                - ""
        "#});

        // in document order; the empty item (the parser collapses `- ""` to
        // null) is kept as "" so S19 can report it
        assert_eq!(dict.extensions.len(), 2);
        assert_eq!(dict.extensions[0].value, "json");
        assert_eq!(dict.extensions[1].value, "");
    }

    #[test]
    fn a_dictionary_without_the_duckdb_section_has_no_extensions() {
        let dict = lower_str("$version: \"0.2.0\"\n");
        assert!(dict.extensions.is_empty());
    }

    // the schema requires `$version`, but lowering must not assume it: a
    // missing key falls through the match's `_` arm to legacy
    #[test]
    fn missing_version_lowers_as_legacy() {
        let dict = lower_str("tables: []\n");
        assert_eq!(dict.format, Format::Legacy);
    }

    #[test]
    fn legacy_document_lowers_without_rich_fields() {
        let dict = lower_str(indoc! {r#"
            $version: "0.1.0"
            tables:
              - name: animals
                columns:
                  - name: weight
                    type: number
        "#});
        assert_eq!(dict.format, Format::Legacy);
        assert!(dict.typedefs.is_empty());
        assert!(dict.source.is_none());
        assert!(dict.tables[0].label.is_none());
        assert!(dict.tables[0].typedefs.is_empty());
    }
}
