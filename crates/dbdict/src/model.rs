//! Typed in-memory model of a data dictionary.
//!
//! Lowered from the source YAML by `lower::lower` once the structural schema
//! has accepted the document, so the lowering code can assume well-formed
//! input. Each significant node carries a `SourceInfo` so schema-check diagnostics
//! can point back at the source.

use quarto_source_map::SourceInfo;

use crate::join_expr::JoinExpr;

#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub value: T,
    pub span: SourceInfo,
}

impl<T> Spanned<T> {
    pub fn new(value: T, span: SourceInfo) -> Self {
        Self { value, span }
    }
}

/// Which spec format the document declared via `$version`. Carried on the
/// lowered dictionary so checks that only make sense for one format (e.g. the
/// coarse-type rules S07/S08/S12–S14) can tell them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `0.1.0` — coarse semantic types (`number`, `string`, …) + parquet.
    Legacy,
    /// `0.2.0` — DuckDB-native types with `typedef:` aliases.
    Rich,
}

/// A named type alias (rich format): `name` expands to the native DuckDB
/// type expression `expr`. An expression may mention other aliases
/// (compounding); resolution order and cycle detection are the validator's
/// job, the model just carries the pairs in document order.
#[derive(Debug, Clone)]
pub struct Typedef {
    pub name: Spanned<String>,
    pub expr: Spanned<String>,
}

/// The rich format's dictionary-level source: the one DuckDB database this
/// dictionary describes. (The legacy format instead has a per-table
/// [`Source`] pointing at a parquet file.)
#[derive(Debug, Clone)]
pub struct DictSource {
    pub span: SourceInfo,
    /// database file path, relative to the dictionary (absolute used as-is)
    pub file: Spanned<String>,
}

#[derive(Debug, Clone)]
pub struct DataDict {
    pub format: Format,
    /// global `typedef:` aliases (rich format), in document order
    pub typedefs: Vec<Typedef>,
    /// dictionary-level `source:` (rich format)
    pub source: Option<DictSource>,
    pub tables: Vec<Table>,
    pub relationships: Vec<Relationship>,
}

impl DataDict {
    /// The first table with the given name, or `None`. Duplicate names are an
    /// error (S10); lookups resolve to the first so downstream checks still run.
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name.value == name)
    }
}

#[derive(Debug, Clone)]
pub struct Table {
    pub name: Spanned<String>,
    /// Optional human-facing display name (rich format).
    pub label: Option<Spanned<String>>,
    /// Table-scoped `typedef:` aliases (rich format); a name here shadows the
    /// same global name for this table's columns.
    pub typedefs: Vec<Typedef>,
    pub columns: Vec<Column>,
    /// Where the table's data lives, when it declares a `source` (legacy
    /// format — the rich format's source lives on [`DataDict`]). Optional
    /// for spec validation; required for metadata validation (M04).
    pub source: Option<Source>,
    /// Spans of the `description`/`details` keys, when present. Held so S16 can
    /// point at a single-table dictionary's misplaced table-level descriptions.
    pub description: Option<SourceInfo>,
    pub details: Option<SourceInfo>,
}

#[derive(Debug, Clone)]
pub struct Source {
    pub span: SourceInfo,
    /// path relative to dictionary
    pub parquet: Spanned<String>,
}

impl Table {
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name.value == name)
    }
}

#[derive(Debug, Clone)]
pub struct Column {
    pub name: Spanned<String>,
    /// Optional human-facing display name (rich format).
    pub label: Option<Spanned<String>>,
    pub constraints: Vec<Spanned<Constraint>>,
    pub col_type: Option<Spanned<String>>,
    pub values: Option<SourceInfo>,
    pub range: Option<Representation>,
    pub examples: Option<Representation>,
    pub units: Option<Spanned<String>>,
    pub time_zone: Option<Spanned<String>>,
}

#[derive(Debug, Clone)]
pub struct Representation {
    pub span: SourceInfo,
    pub items: Vec<Spanned<Scalar>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Number(f64),
    String(String), // includes date/times
    Bool(bool),
    Null,
    /// A list or map — never valid in a representation list.
    Compound,
}

impl Scalar {
    /// English noun phrase naming the scalar's kind, for diagnostics.
    pub fn noun(&self) -> &'static str {
        match self {
            Scalar::Number(_) => "a number",
            Scalar::String(_) => "a string",
            Scalar::Bool(_) => "a boolean",
            Scalar::Null => "null",
            Scalar::Compound => "a list or map",
        }
    }
}

impl Column {
    pub fn has(&self, c: Constraint) -> bool {
        self.constraints.iter().any(|x| x.value == c)
    }

    /// True if the column is unique-by-row: explicitly `unique` or
    /// `primary_key` (which the spec defines as implying `unique`).
    pub fn is_unique_implied(&self) -> bool {
        self.has(Constraint::Unique) || self.has(Constraint::PrimaryKey)
    }

    /// True if the column may not contain nulls: explicitly `required` or
    /// `primary_key` (which the spec defines as implying `required`).
    pub fn is_required_implied(&self) -> bool {
        self.has(Constraint::Required) || self.has(Constraint::PrimaryKey)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constraint {
    PrimaryKey,
    ForeignKey,
    Required,
    Unique,
}

impl Constraint {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "primary_key" => Self::PrimaryKey,
            "foreign_key" => Self::ForeignKey,
            "required" => Self::Required,
            "unique" => Self::Unique,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Relationship {
    pub cardinality: Spanned<Cardinality>,
    /// The original join string with its source span. Kept alongside the
    /// parsed `JoinExpr` so diagnostics about parse failure can refer back to
    /// it.
    pub join_text: Spanned<String>,
    /// `None` if the join string failed to parse — S04 is emitted in that
    /// case and downstream rules that need the parsed form (S01, S05,
    /// S06) skip the relationship.
    pub join: Option<JoinExpr>,
    pub conflicts: Vec<Spanned<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    OneToOne,
    OneToMany,
    ManyToOne,
}

impl Cardinality {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "one-to-one" => Self::OneToOne,
            "one-to-many" => Self::OneToMany,
            "many-to-one" => Self::ManyToOne,
            _ => return None,
        })
    }
}
