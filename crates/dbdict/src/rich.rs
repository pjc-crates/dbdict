//! Rich-format (0.2.0) metadata validation: the duckdb round-trip seam.
//!
//! The rich format compares DESCRIBE-to-DESCRIBE. The dictionary is
//! instantiated in a scratch in-memory database (`CREATE TYPE` for typedefs,
//! `CREATE TABLE` for tables) and DESCRIBEd, giving the *expected* canonical
//! type of every typed column; the real database named by the dictionary's
//! `source` is DESCRIBEd, giving the *actual* ones; and the two sides are
//! diffed as plain strings. DuckDB canonicalizes both sides identically
//! (proven by the phase-1 spike), so the comparison is exact — no type
//! algebra in this crate.
//!
//! Core stays duckdb-free: [`DuckdbBackend`] is the seam, implemented by the
//! `dbdict-duckdb` crate with the native (bundled) duckdb library, and passed
//! in by the caller (the CLI). Tests drive this module with a fake backend.

use std::path::Path;

use quarto_source_map::SourceInfo;

use crate::model::{DataDict, Scalar, Table};
use crate::problem::{ProblemKind, ProblemSet, Severity};
use crate::validate_spec::{is_infinite, parse_date, parse_datetime, parse_naive_datetime};

/// One relation as `DESCRIBE` reports it: the name and the
/// `(column, canonical type)` pairs in table order.
#[derive(Debug, Clone)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<(String, String)>,
}

/// The outcome of instantiating a dictionary in a scratch database.
#[derive(Debug, Clone)]
pub struct Instantiated {
    /// Per dictionary table, in dictionary order: the scratch DESCRIBE of its
    /// *typed* columns. Untyped columns make no type claim and are absent; a
    /// column whose `type:` failed to instantiate is also absent (and named in
    /// `failures` instead), so a table's vector holds only the columns that
    /// canonicalized. Same length as `dict.tables`.
    pub tables: Vec<Vec<(String, String)>>,
    /// Everything that failed to instantiate, each with duckdb's own error
    /// text. Indices point into the dictionary model; the caller maps them
    /// back to source spans.
    pub failures: Vec<InstantiateFailure>,
}

/// One thing duckdb rejected while building the scratch database.
#[derive(Debug, Clone)]
pub enum InstantiateFailure {
    /// `CREATE TYPE` failed for a typedef — unknown or cyclic reference,
    /// malformed expression. `table` is `None` for a global typedef, else the
    /// index of the table whose scoped list `index` points into.
    Typedef {
        table: Option<usize>,
        index: usize,
        error: String,
    },
    /// A column's `type:` expression was rejected when creating its table.
    Column {
        table: usize,
        column: usize,
        error: String,
    },
}

/// How a canonical duckdb type behaves, as far as the descriptive-key checks
/// care: which representation keys make sense on a column of this type.
/// Assigned by the backend ([`DuckdbBackend::classify`]) — the type-spelling
/// knowledge stays with duckdb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCategory {
    Boolean,
    /// integers, floats, decimals — orderable, and the home of `units`
    Numeric,
    /// `ENUM(...)` — the type itself lists the categories
    Enum,
    /// `DATE`
    Date,
    /// zoneless `TIMESTAMP` (any precision) — bounds are written naive
    Timestamp,
    /// `TIMESTAMP WITH TIME ZONE` — bounds carry their own offset
    TimestampTz,
    /// everything else: strings, blobs, nested/composite types, …
    Other,
}

/// What the rich metadata level asks of duckdb. Implemented by
/// `dbdict-duckdb`; faked in core tests.
pub trait DuckdbBackend {
    /// Build the scratch database from the dictionary's typedefs and tables,
    /// and DESCRIBE what was created.
    fn instantiate(&self, dict: &DataDict) -> Instantiated;

    /// Read the real database's schema: every table with its columns, as
    /// DESCRIBE canonicalizes them. `Err` is the human-readable reason the
    /// database could not be opened or listed.
    fn read_schema(&self, db_file: &Path) -> Result<Vec<TableSchema>, String>;

    /// Classify a canonical type (as returned by [`Self::instantiate`]) for
    /// the descriptive-key checks (S07/S08/S12–S14 in rich mode).
    fn classify(&self, canonical_type: &str) -> TypeCategory;
}

/// The rich twin of the legacy parquet comparison: resolve the dictionary's
/// `source`, read the real schema, instantiate the dictionary, and diff.
pub(crate) fn check_meta(
    dict_path: &Path,
    dict: &DataDict,
    table: Option<&str>,
    duckdb: &dyn DuckdbBackend,
    out: &mut ProblemSet,
) {
    // resolve the `--table` filter exactly like the legacy path: an unknown
    // name is the same TableNotFound pre-flight. only the outcome is reused —
    // the loops below re-apply the filter by name because they also need each
    // table's *index* to reach into the instantiation results
    if crate::select_tables(dict, table, out).is_none() {
        return;
    }

    // dictionary-side coherence first — instantiation failures (M08/M09) and
    // the descriptive-key checks — so the dictionary's own problems report
    // even when its database is missing or unreadable
    let instantiated = duckdb.instantiate(dict);
    report_instantiate_failures(dict, &instantiated.failures, table, out);
    for (i, dict_table) in dict.tables.iter().enumerate() {
        if !table_selected(table, &dict_table.name.value) {
            continue;
        }
        if let Some(expected) = instantiated.tables.get(i) {
            check_descriptive_keys(dict_table, expected, duckdb, out);
        }
    }

    let Some(source) = &dict.source else {
        // M04: `source` is optional at the spec level but required here. there
        // is no node to point at, so the problem is unlocated (but still coded,
        // unlike a bare `preflight`)
        out.push(crate::Problem::unlocated(
            ProblemKind::MissingSource,
            Severity::Error,
            "A dictionary validated against data must declare a `source`.",
            "the dictionary declares no `source`, so there is no database to validate against",
        ));
        return;
    };
    // `file` is resolved relative to the dictionary; `join` keeps an
    // absolute path as-is, which is exactly the documented behaviour
    let base_dir = dict_path.parent().unwrap_or_else(|| Path::new(""));
    let db_file = base_dir.join(&source.file.value);
    let actual = match duckdb.read_schema(&db_file) {
        Ok(actual) => actual,
        Err(reason) => {
            // M05: the database the dictionary names can't be opened; duckdb's
            // own reason is the message
            out.push_located(
                ProblemKind::UnreadableSource,
                Severity::Error,
                "A dictionary's `source` must point at a readable DuckDB database.",
                reason,
                [source.span.clone(), source.file.span.clone()],
            );
            return;
        }
    };
    for (i, dict_table) in dict.tables.iter().enumerate() {
        if !table_selected(table, &dict_table.name.value) {
            continue;
        }
        // M06: the dictionary describes a table the database lacks. checked
        // before the instantiation gate — a table that failed to instantiate
        // is still documented, so its absence from the database still matters.
        // matched case-insensitively (duckdb identifiers are)
        let Some(actual_table) = actual
            .iter()
            .find(|a| names_eq(&a.name, &dict_table.name.value))
        else {
            out.push_located(
                ProblemKind::MissingTable,
                Severity::Error,
                "Every table in the dictionary must be present in the database.",
                "is missing from the database",
                [dict_table.name.span.clone()],
            );
            continue;
        };
        let Some(expected) = instantiated.tables.get(i) else {
            continue;
        };
        diff_columns(dict_table, expected, &actual_table.columns, out);
        report_undocumented_columns(dict_table, &actual_table.columns, out);
    }
    // M07: database tables the dictionary does not describe. they exist only
    // in the database, so like M03 they are named in the message rather than
    // located in source. skipped under `--table`, which asks about one
    // dictionary table, not the database's coverage
    if table.is_some() {
        return;
    }
    for actual_table in &actual {
        // case-insensitive: a db table matches a dict table of any case
        let documented = dict
            .tables
            .iter()
            .any(|t| names_eq(&t.name.value, &actual_table.name));
        if !documented {
            out.push(crate::Problem::unlocated(
                ProblemKind::ExtraTable,
                Severity::Warning,
                "Every table in the database should be described in the dictionary.",
                format!(
                    "`{}` is in the database but not the dictionary",
                    actual_table.name
                ),
            ));
        }
    }
}

/// Whether a dictionary name and a database name refer to the same object.
/// DuckDB identifiers are case-insensitive (but case-preserving in `DESCRIBE`),
/// so a lowercase `dbdict.yaml` matches a `CamelCase` database. ASCII folding
/// covers ASCII identifiers — the practical universe; duckdb's own Unicode
/// folding is not replicated here.
fn names_eq(dict_name: &str, db_name: &str) -> bool {
    dict_name.eq_ignore_ascii_case(db_name)
}

/// Whether a dictionary table is in scope given the `--table` filter (matched
/// exactly, like the [`crate::select_tables`] pre-flight — this compares the
/// user's flag against a dictionary name, not a database name).
fn table_selected(filter: Option<&str>, name: &str) -> bool {
    filter.is_none() || filter == Some(name)
}

/// The rich rework of S07/S08/S12–S14: the descriptive keys (`values`,
/// `range`, `examples`, `units`, `time_zone`) checked against the column's
/// *canonicalized* type, classified by the backend.
///
/// Unlike the legacy rules, nothing is *required*: the coarse vocabulary
/// carried intent (`number(quantity)` vs `number(id)`) that a bare duckdb
/// type cannot — a `BIGINT` may be a measure or an identifier — so these
/// checks only reject combinations that cannot make sense. They run at the
/// metadata level (not validate-spec) because canonicalization needs the
/// scratch database; the rule codes keep their spec identities.
fn check_descriptive_keys(
    dict_table: &Table,
    expected: &[(String, String)],
    duckdb: &dyn DuckdbBackend,
    out: &mut ProblemSet,
) {
    for column in &dict_table.columns {
        // untyped columns make no claims; columns whose type failed to
        // instantiate are already reported (M09) and have no canonical form
        let Some(canonical) = expected
            .iter()
            .find(|(name, _)| name == &column.name.value)
            .map(|(_, t)| t.as_str())
        else {
            continue;
        };
        let category = duckdb.classify(canonical);
        let at = |span: &SourceInfo| {
            [
                dict_table.name.span.clone(),
                column.name.span.clone(),
                span.clone(),
            ]
        };

        // S07: a boolean speaks for itself; `range` needs an orderable type
        let orderable = matches!(
            category,
            TypeCategory::Numeric
                | TypeCategory::Date
                | TypeCategory::Timestamp
                | TypeCategory::TimestampTz
        );
        if category == TypeCategory::Boolean {
            for (span, key) in [
                (column.values.as_ref(), "values"),
                (column.range.as_ref().map(|r| &r.span), "range"),
                (column.examples.as_ref().map(|e| &e.span), "examples"),
            ] {
                if let Some(span) = span {
                    out.push_spec_error(
                        "S07",
                        "A `BOOLEAN` column must not have `values`, `range`, or `examples`.",
                        format!("has type `{canonical}` but uses `{key}`"),
                        at(span),
                    );
                }
            }
        } else if let Some(range) = &column.range
            && !orderable
        {
            let expected_rule = if category == TypeCategory::Enum {
                format!(
                    "An `{canonical}` column lists its categories in its type; it must not use `range`."
                )
            } else {
                format!("A `{canonical}` column is not orderable, so it must not use `range`.")
            };
            out.push_spec_error(
                "S07",
                expected_rule,
                format!("has type `{canonical}` but uses `range`"),
                at(&range.span),
            );
        }

        // S08: units annotate a magnitude, so they need a numeric type
        if let Some(units) = &column.units
            && category != TypeCategory::Numeric
        {
            out.push_spec_error(
                "S08",
                "A column with `units` must have a numeric type.",
                format!("has `units` but its type is `{canonical}`"),
                at(&units.span),
            );
        }

        // S14: a time zone annotates a timestamp
        if let Some(time_zone) = &column.time_zone
            && !matches!(
                category,
                TypeCategory::Timestamp | TypeCategory::TimestampTz
            )
        {
            out.push_spec_error(
                "S14",
                "A column with `time_zone` must have a timestamp type.",
                format!("has `time_zone` but its type is `{canonical}`"),
                at(&time_zone.span),
            );
        }

        // S12 then S13 on an orderable range: bounds must parse for the
        // category before comparing them for order means anything
        if orderable && let Some(range) = &column.range {
            let mut bounds_ok = true;
            for bound in &range.items {
                if bound_matches_category(category, &bound.value) {
                    continue;
                }
                bounds_ok = false;
                out.push_spec_error(
                    "S12",
                    format!(
                        "Each `range` value of a `{canonical}` column must be {}.",
                        category_noun(category)
                    ),
                    format!("is {}", bound.value.noun()),
                    at(&bound.span),
                );
            }
            if bounds_ok
                && range.items.len() == 2
                && range_descending(category, &range.items[0].value, &range.items[1].value)
            {
                out.push_spec_error(
                    "S13",
                    "A range's minimum must be less than or equal to its maximum.",
                    "is greater than the maximum",
                    at(&range.items[0].span),
                );
            }
        }
    }
}

/// Whether one range bound fits the column's category. An infinite bound
/// leaves that end open on any orderable type.
///
/// This mirrors the legacy `validate_spec::value_matches_type` /
/// `range_descending` (keyed on coarse type names rather than `TypeCategory`);
/// the two vocabularies are separate enough that a shared abstraction would be
/// more tangle than saving — keep the two in step by hand.
fn bound_matches_category(category: TypeCategory, bound: &Scalar) -> bool {
    if is_infinite(bound) {
        return true;
    }
    match category {
        TypeCategory::Numeric => matches!(bound, Scalar::Number(_)),
        TypeCategory::Date => matches!(bound, Scalar::String(s) if parse_date(s).is_some()),
        // zoneless timestamps take naive bounds (a `time_zone` key names
        // their zone); zoned ones carry each bound's own offset
        TypeCategory::Timestamp => {
            matches!(bound, Scalar::String(s) if parse_naive_datetime(s).is_some())
        }
        TypeCategory::TimestampTz => {
            matches!(bound, Scalar::String(s) if parse_datetime(s).is_some())
        }
        _ => true,
    }
}

/// English noun phrase for what a category's range bounds look like.
fn category_noun(category: TypeCategory) -> &'static str {
    match category {
        TypeCategory::Numeric => "a number",
        TypeCategory::Date => "an ISO 8601 date (YYYY-MM-DD)",
        TypeCategory::Timestamp => "a zoneless ISO 8601 datetime (e.g. 2024-01-31T09:30:00)",
        TypeCategory::TimestampTz => {
            "an ISO 8601 datetime with a timezone (e.g. 2024-01-31T09:30:00Z)"
        }
        _ => "a value",
    }
}

/// Whether `lo`..`hi` runs backwards for the category. Both bounds are known
/// to parse (S12 ran first). An infinite bound orders as `-inf` < any value
/// < `+inf`, so it runs backwards only on the wrong end.
fn range_descending(category: TypeCategory, lo: &Scalar, hi: &Scalar) -> bool {
    if is_infinite(lo) || is_infinite(hi) {
        let is_pos = |v: &Scalar| matches!(v, Scalar::Number(f) if *f == f64::INFINITY);
        let is_neg = |v: &Scalar| matches!(v, Scalar::Number(f) if *f == f64::NEG_INFINITY);
        return (is_pos(lo) && !is_pos(hi)) || (is_neg(hi) && !is_neg(lo));
    }
    match (category, lo, hi) {
        (TypeCategory::Numeric, Scalar::Number(a), Scalar::Number(b)) => a > b,
        (TypeCategory::Date, Scalar::String(a), Scalar::String(b)) => {
            match (parse_date(a), parse_date(b)) {
                (Some(a), Some(b)) => a > b,
                _ => false,
            }
        }
        (TypeCategory::Timestamp, Scalar::String(a), Scalar::String(b)) => {
            match (parse_naive_datetime(a), parse_naive_datetime(b)) {
                (Some(a), Some(b)) => a > b,
                _ => false,
            }
        }
        (TypeCategory::TimestampTz, Scalar::String(a), Scalar::String(b)) => {
            match (parse_datetime(a), parse_datetime(b)) {
                (Some(a), Some(b)) => a > b,
                _ => false,
            }
        }
        _ => false,
    }
}

/// M03 — database columns the dictionary does not describe. Reported against
/// the *dictionary* column list (not the scratch DESCRIBE, which omits
/// untyped columns that are nevertheless documented).
fn report_undocumented_columns(
    dict_table: &Table,
    actual: &[(String, String)],
    out: &mut ProblemSet,
) {
    for (name, actual_type) in actual {
        // case-insensitive: a db column matches a documented column of any case
        let documented = dict_table
            .columns
            .iter()
            .any(|c| names_eq(&c.name.value, name));
        if !documented {
            out.push(crate::Problem::undocumented_column(
                name,
                actual_type.clone(),
            ));
        }
    }
}

/// Map everything duckdb rejected while building the scratch database back to
/// the dictionary source: M08 at the failing typedef, M09 at the failing
/// column `type:`. The backend reports by index; the model supplies the spans.
/// Out-of-range indices would be a backend bug and are silently skipped —
/// there is no source location to report them at. Failures scoped to a table
/// the `--table` filter deselects are skipped; global typedef failures always
/// report.
fn report_instantiate_failures(
    dict: &DataDict,
    failures: &[InstantiateFailure],
    table_filter: Option<&str>,
    out: &mut ProblemSet,
) {
    for failure in failures {
        match failure {
            InstantiateFailure::Typedef {
                table,
                index,
                error,
            } => {
                // a global typedef lives on the dictionary; a scoped one on
                // its table, which is named as enclosing context
                let (typedefs, table_span) = match table {
                    None => (&dict.typedefs, None),
                    Some(t) => {
                        let Some(dict_table) = dict.tables.get(*t) else {
                            continue;
                        };
                        if !table_selected(table_filter, &dict_table.name.value) {
                            continue;
                        }
                        (&dict_table.typedefs, Some(dict_table.name.span.clone()))
                    }
                };
                let Some(typedef) = typedefs.get(*index) else {
                    continue;
                };
                let mut spans = Vec::new();
                spans.extend(table_span);
                spans.push(typedef.name.span.clone());
                spans.push(typedef.expr.span.clone());
                out.push_located(
                    ProblemKind::InvalidTypedef,
                    Severity::Error,
                    "A typedef must be a DuckDB type expression that instantiates.",
                    error.clone(),
                    spans,
                );
            }
            InstantiateFailure::Column {
                table,
                column,
                error,
            } => {
                let Some(dict_table) = dict.tables.get(*table) else {
                    continue;
                };
                if !table_selected(table_filter, &dict_table.name.value) {
                    continue;
                }
                let Some(dict_column) = dict_table.columns.get(*column) else {
                    continue;
                };
                let type_span = dict_column
                    .col_type
                    .as_ref()
                    .map_or_else(|| dict_column.name.span.clone(), |t| t.span.clone());
                out.push_located(
                    ProblemKind::InvalidColumnType,
                    Severity::Error,
                    "A column's `type` must be a DuckDB type expression or typedef alias that \
                     instantiates.",
                    error.clone(),
                    [
                        dict_table.name.span.clone(),
                        dict_column.name.span.clone(),
                        type_span,
                    ],
                );
            }
        }
    }
}

/// Compare one dictionary table's columns against the database: expected
/// canonical types come from the scratch DESCRIBE, actual ones from the real
/// database, and equality is exact string comparison — duckdb canonicalized
/// both sides.
fn diff_columns(
    dict_table: &Table,
    expected: &[(String, String)],
    actual: &[(String, String)],
    out: &mut ProblemSet,
) {
    for column in &dict_table.columns {
        // the db side is matched case-insensitively (duckdb identifiers);
        // the scratch `expected` side carries the dictionary's own spelling,
        // so it matches exactly
        let actual_type = actual
            .iter()
            .find(|(name, _)| names_eq(&column.name.value, name))
            .map(|(_, t)| t);
        let expected_type = expected
            .iter()
            .find(|(name, _)| name == &column.name.value)
            .map(|(_, t)| t);
        // M02: a documented column the database lacks, typed or not
        let Some(actual_type) = actual_type else {
            out.push_located(
                ProblemKind::MissingInData,
                Severity::Error,
                "Every column in the dictionary must be present in the database.",
                "is missing from the database",
                [dict_table.name.span.clone(), column.name.span.clone()],
            );
            continue;
        };
        // a column with no `type:` makes no type claim, and one whose type
        // failed to instantiate is already reported — either way there is no
        // expected side to compare
        let Some(expected_type) = expected_type else {
            continue;
        };
        if actual_type != expected_type {
            // M01. the dictionary may spell the type as an alias, so the
            // message states the canonical expansion the comparison used
            let type_span = column
                .col_type
                .as_ref()
                .map_or_else(|| column.name.span.clone(), |t| t.span.clone());
            out.push_located(
                ProblemKind::TypeMismatch {
                    declared: expected_type.clone(),
                    actual: actual_type.clone(),
                },
                Severity::Error,
                "A column's declared type must match the database.",
                format!("instantiates to `{expected_type}` but the database has `{actual_type}`"),
                [
                    dict_table.name.span.clone(),
                    column.name.span.clone(),
                    type_span,
                ],
            );
        }
    }
}
