//! Spec-level validation, the `S##` checks (see `site/validation.md`).
//!
//! [`validate_spec`] runs two internal passes — a split not surfaced in the CLI:
//!
//! 1. **schema**: structural validation against the embedded `schema.yaml` via
//!    the `quarto-yaml-validation` crate — everything a JSON Schema can express.
//! 2. **spec**: the cross-table semantic checks below that the schema can't
//!    express (foreign-key targets, `join` parsing, cardinality, …).
//!
//! The second pass only runs if the first succeeds: there is no point chasing
//! FK references in a document whose `tables` block is malformed.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime};
use quarto_source_map::SourceInfo;
use quarto_yaml::YamlWithSourceInfo;
use quarto_yaml_validation::error::ValidationErrorKind;
use quarto_yaml_validation::{Schema, SchemaRegistry, ValidationDiagnostic, ValidationError};

use crate::join_expr::{JoinExpr, QCol};
use crate::model::{Cardinality, Column, DataDict, Format, Scalar, Spanned, Table};
use crate::problem::{Problem, ProblemKind, ProblemSet, Suggestion, subspan};
use crate::{SourceContext, lower};

/// The canonical documentation URL suggested for `$learn_more` — this fork's
/// repository (upstream used its published site; dbdict has no published site
/// yet, and the repo URL stays valid either way)
pub const LEARN_MORE_URL: &str = "https://github.com/pjc-wspace/dbdict";

// each spec version has its own embedded schema; `load` picks one by peeking
// at the document's `$version` (see `select_schema`)
const LEGACY_SCHEMA_YAML: &str = include_str!("../../../schema.yaml");
const RICH_SCHEMA_YAML: &str = include_str!("../../../schema-0.2.yaml");

fn compile_schema(schema_yaml: &str) -> Schema {
    let yaml = quarto_yaml::parse(schema_yaml).expect("embedded schema must be parseable YAML");
    Schema::from_yaml(&yaml).expect("embedded schema must compile to a valid schema")
}

/// The legacy (0.1.0) schema: coarse types, per-table parquet source.
fn legacy_schema() -> &'static Schema {
    // OnceLock: compile the schema on first use, then share the single
    // compiled copy across threads — a `static` can't hold a lazily-built
    // value directly
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| compile_schema(LEGACY_SCHEMA_YAML))
}

/// The rich (0.2.0) schema: free-form DuckDB types, typedef aliases.
fn rich_schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| compile_schema(RICH_SCHEMA_YAML))
}

/// A `$version` the document declares but this tool does not support: the
/// value (rendered for the message) and its span.
struct UnsupportedVersion {
    version: String,
    span: SourceInfo,
}

/// The schema `doc` asks to be validated against: the document declares its
/// own format via `$version` — `0.1.0` legacy, `0.2.0` rich. Only a truly
/// *absent* `$version` falls through to the legacy schema (whose required-key
/// error reports it); any other value, string or not, takes the
/// unsupported-version path. Catching non-strings here matters because an
/// unquoted `$version: 0.2` is a YAML float — letting it hit the legacy
/// schema would report the misleading "must be 0.1.0" enum error.
// the Err carries a SourceInfo, making the Result large enough for clippy to
// suggest boxing; called once per validation, so readability wins over size
#[allow(clippy::result_large_err)]
fn select_schema(doc: &YamlWithSourceInfo) -> Result<&'static Schema, UnsupportedVersion> {
    let Some(version) = doc.get_hash_value("$version") else {
        return Ok(legacy_schema());
    };
    match version.yaml.as_str() {
        Some("0.1.0") => Ok(legacy_schema()),
        Some("0.2.0") => Ok(rich_schema()),
        _ => Err(UnsupportedVersion {
            version: version_text(version),
            span: version.source_info.clone(),
        }),
    }
}

/// Render a `$version` value for the unsupported-version message. `$version`
/// should be a string; a number, boolean, or null arrives when the value was
/// written unquoted, so spell those out rather than showing nothing.
fn version_text(version: &YamlWithSourceInfo) -> String {
    let yaml = &version.yaml;
    if let Some(s) = yaml.as_str() {
        s.to_string()
    } else if let Some(i) = yaml.as_i64() {
        i.to_string()
    } else if let Some(f) = yaml.as_f64() {
        f.to_string()
    } else if let Some(b) = yaml.as_bool() {
        b.to_string()
    } else if yaml.is_null() {
        "null".to_string()
    } else {
        // a list or mapping — there is no scalar to quote
        "this value".to_string()
    }
}

/// Validate the `dbdict.yaml` file at `path`. The returned [`ProblemSet`]
/// bundles every problem (errors and warnings, in source order) with the source
/// context needed to render them; [`ProblemSet::status`] reports whether the
/// document is valid. Failures that prevent checking altogether — I/O,
/// unparseable YAML, a structurally invalid document — surface as pre-flight
/// [`Problem`]s in the set.
pub fn validate_spec(path: &Path) -> ProblemSet {
    let (mut problems, doc) = match load(path) {
        Ok(loaded) => loaded,
        Err(problems) => return problems,
    };
    // We only want the problems here, not the lowered dictionary.
    validate_and_lower(&doc, &mut problems);
    problems
}

/// Read, parse, and schema-check the document at `path`, creating the run's
/// [`ProblemSet`] with the document's source — this is where every level starts.
/// `Ok((problems, doc))` hands back the fresh set and the parsed AST to validate;
/// `Err(problems)` carries a pre-flight failure (I/O, unparseable YAML, or a
/// document the schema rejects) for which no document could be produced.
pub(crate) fn load(path: &Path) -> Result<(ProblemSet, YamlWithSourceInfo), ProblemSet> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) => return Err(ProblemSet::from_preflight(ProblemKind::Io, e.to_string())),
    };
    let filename = path.display().to_string();

    let doc = match quarto_yaml::parse_file(&content, &filename) {
        Ok(doc) => doc,
        Err(e) => {
            return Err(ProblemSet::from_preflight(
                ProblemKind::Parse,
                e.to_string(),
            ));
        }
    };

    let mut source = SourceContext::new();
    let file_id = quarto_yaml::file_id_for_filename(&filename);
    source.add_file_with_id(file_id, filename, Some(content));

    // unsupported version: report with the value's span and stop — validating
    // against either schema would produce a misleading enum error
    let schema = match select_schema(&doc) {
        Ok(schema) => schema,
        Err(unsupported) => {
            let mut problems = ProblemSet::new(source);
            problems.push(Problem::schema(
                "version",
                format!("`{}` is not a supported spec version", unsupported.version),
                Some(unsupported.span),
                Some("Supported versions: \"0.1.0\" (legacy), \"0.2.0\" (rich/duckdb).".into()),
            ));
            return Err(problems);
        }
    };

    let registry = SchemaRegistry::new();
    if let Err(err) = quarto_yaml_validation::validate(&doc, schema, &registry, &source) {
        // Lift the structural error into our own vocabulary so it renders through
        // the annotate-snippets pipeline like every other diagnostic, rather than
        // the validator's own (ariadne) text.
        let diagnostic = ValidationDiagnostic::from_validation_error(&err, &source);
        let span = schema_error_span(&err);
        let hints = diagnostic.hints();
        let hint = (!hints.is_empty()).then(|| hints.join(" "));
        let mut problems = ProblemSet::new(source);
        problems.push(Problem::schema(err.error_code(), err.message(), span, hint));
        return Err(problems);
    }

    Ok((ProblemSet::new(source), doc))
}

/// The tightest span for a structural error. The validator points an unknown
/// property at its enclosing object; narrow that to the offending key so the
/// annotation lands on the property itself. Other errors keep the node the
/// validator attached.
fn schema_error_span(err: &ValidationError) -> Option<SourceInfo> {
    let node = err.yaml_node.as_ref()?;
    if let ValidationErrorKind::UnknownProperty { property } = &err.kind
        && let Some(entry) = node.as_hash().and_then(|entries| {
            entries
                .iter()
                .find(|e| e.key.yaml.as_str() == Some(property))
        })
    {
        return Some(entry.key_span.clone());
    }
    Some(node.source_info.clone())
}

/// Lower the parsed document `doc` and run the S## semantic checks, pushing any
/// findings into `out`. Returns the lowered dictionary when the spec validates,
/// or `None` when it has errors (which `out` then carries).
pub(crate) fn validate_and_lower(
    doc: &YamlWithSourceInfo,
    out: &mut ProblemSet,
) -> Option<DataDict> {
    let dict = lower::lower(doc, out);
    check_spec(&dict, out);
    validate_s09_learn_more(doc, out);
    validate_s17_version(doc, out);
    out.sort();

    if out.status().failed() {
        None
    } else {
        Some(dict)
    }
}

/// Run every rule, pushing any findings into `out`; call [`ProblemSet::sort`]
/// afterwards to put the findings in source order.
///
/// Relationship-level rules run against the whole dictionary. Column-level rules
/// run per column from a single iteration here, sequenced so a more specific
/// check runs only when the general one it refines passed: a malformed `name`
/// blocks the uniqueness check, and the representation chain narrows from "the
/// right key is present" (S07) to "its values have the right type" (S12) to "the
/// range is ordered" (S13).
fn check_spec(dict: &DataDict, out: &mut ProblemSet) {
    validate_s02_relationship_table_refs(dict, out);
    validate_s03_relationship_column_refs(dict, out);
    validate_s04_join_table_count(dict, out);
    validate_s05_conflicts_present_on_both_sides(dict, out);
    validate_s06_cardinality_consistency(dict, out);
    validate_s16_single_table_description(dict, out);

    // rich names live in duckdb, whose identifiers are case-insensitive: two
    // names differing only in ASCII case cannot coexist in the database, so
    // S10 folds case for rich documents (matching `names_eq` at the meta
    // level). legacy names live in parquet, where names are case-sensitive,
    // so the legacy format keeps exact matching
    let fold_case = dict.format == Format::Rich;
    let mut seen_tables: HashMap<String, SourceInfo> = HashMap::new();
    for table in &dict.tables {
        if validate_s11_table_name(table, out) {
            validate_s10_unique_table_name(table, fold_case, &mut seen_tables, out);
        }
        let mut seen: HashMap<String, SourceInfo> = HashMap::new();
        for col in &table.columns {
            // S07/S08/S12–S14 are written against the legacy coarse type
            // vocabulary (`number(quantity)`, `datetime`, …), which the rich
            // format replaces with duckdb type expressions. skipped for rich
            // documents until they are reworked to classify duckdb types
            // (planned alongside the round-trip validation seam). gated *in
            // place* rather than hoisted: two problems at the same span keep
            // their push order when sorted, so moving a call would reorder
            // legacy diagnostics
            let coarse_types = dict.format == Format::Legacy;
            validate_s01_foreign_key(dict, table, col, out);
            if coarse_types {
                validate_s08_units(table, col, out);
                validate_s14_time_zone(table, col, out);
            }
            validate_s15_time_zone_format(table, col, out);
            if validate_s11_column_name(table, col, out) {
                validate_s10_unique_name(table, col, fold_case, &mut seen, out);
            }
            if coarse_types
                && validate_s07_representation(table, col, out)
                && validate_s12_value_types(table, col, out)
            {
                validate_s13_range_order(table, col, out);
            }
        }
    }
}

// --- S02 --------------------------------------------------------------

fn validate_s02_relationship_table_refs(dict: &DataDict, out: &mut ProblemSet) {
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };
        for q in join.qcols() {
            if dict.table(&q.table).is_none() {
                let span = subspan(&rel.join_text.span, q.start, q.end)
                    .unwrap_or_else(|| rel.join_text.span.clone());
                out.push_spec_error(
                    "S02",
                    "A `join` must refer to known tables.",
                    format!("table `{}` is not defined", q.table),
                    [span],
                );
            }
        }
    }
}

// --- S03 --------------------------------------------------------------

fn validate_s03_relationship_column_refs(dict: &DataDict, out: &mut ProblemSet) {
    for rel in &dict.relationships {
        if let Some(join) = &rel.join {
            for q in join.qcols() {
                // Skip if the table doesn't exist — S02 handles that case
                // and a column report would be noise.
                let Some(table) = dict.table(&q.table) else {
                    continue;
                };
                if table.column(&q.column).is_none() {
                    let span = subspan(&rel.join_text.span, q.start, q.end)
                        .unwrap_or_else(|| rel.join_text.span.clone());
                    out.push_spec_error(
                        "S03",
                        "A `join` must refer to known columns.",
                        format!("table `{}` has no column `{}`", q.table, q.column),
                        [span],
                    );
                }
            }
        }
        // `conflicts` column references are checked by S05 alongside the
        // "appears on both sides" check, so a missing column there reports
        // the more specific message.
    }
}

// --- S04 --------------------------------------------------------------

fn validate_s04_join_table_count(dict: &DataDict, out: &mut ProblemSet) {
    // Parse failures are emitted during lowering. Here we only check the
    // table-count invariant on successfully parsed joins.
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };
        let tables = join.tables();
        if tables.is_empty() || tables.len() > 2 {
            out.push_spec_error(
                "S04",
                "A `join` must reference one (self-join) or two tables.",
                format!("this `join` references {} tables", tables.len()),
                [rel.join_text.span.clone()],
            );
        }
    }
}

// --- S01 --------------------------------------------------------------

fn validate_s01_foreign_key(dict: &DataDict, table: &Table, col: &Column, out: &mut ProblemSet) {
    use crate::model::Constraint::*;

    if !col.has(ForeignKey) {
        return;
    }
    // the FK column must be paired with a `primary_key` column by an
    // *equality* conjunct — the same resolution D04 queries against, shared
    // via `foreign_key_targets` so the two checks cannot drift
    let satisfied = !dict
        .foreign_key_targets(&table.name.value, &col.name.value)
        .is_empty();
    if !satisfied {
        let fk_span = col
            .constraints
            .iter()
            .find(|c| c.value == ForeignKey)
            .map_or_else(|| col.name.span.clone(), |c| c.span.clone());
        out.push_spec_error(
            "S01",
            "Every `foreign_key` column must have a matching relationship to a `primary_key`.",
            "is `foreign_key` but no relationship points it at a `primary_key`",
            [table.name.span.clone(), col.name.span.clone(), fk_span],
        );
    }
}

// --- S05 --------------------------------------------------------------

fn validate_s05_conflicts_present_on_both_sides(dict: &DataDict, out: &mut ProblemSet) {
    for rel in &dict.relationships {
        if rel.conflicts.is_empty() {
            continue;
        }
        let Some(join) = &rel.join else { continue };
        let tables = join.tables();
        // For a self-join, the "both sides" reduces to the single table; for
        // a normal join, both tables must contain the column.
        for c in &rel.conflicts {
            let mut missing_from: Vec<&str> = Vec::new();
            for t_name in &tables {
                let Some(table) = dict.table(t_name) else {
                    // S02 already flagged the missing table; skip to avoid
                    // a cascade of confusing reports.
                    continue;
                };
                if table.column(&c.value).is_none() {
                    missing_from.push(*t_name);
                }
            }
            if !missing_from.is_empty() {
                out.push_spec_error(
                    "S05",
                    "A `conflicts` entry must name a column on both sides of the join.",
                    format!(
                        "`{}` is not a column of {}",
                        c.value,
                        join_with_commas(&missing_from)
                    ),
                    [rel.join_text.span.clone(), c.span.clone()],
                );
            }
        }
    }
}

fn join_with_commas(items: &[&str]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("`{s}`")).collect();
    match quoted.len() {
        0 => String::new(),
        1 => quoted[0].clone(),
        _ => {
            let (last, init) = quoted.split_last().unwrap();
            format!("{} and {}", init.join(", "), last)
        }
    }
}

// --- S06 --------------------------------------------------------------

fn validate_s06_cardinality_consistency(dict: &DataDict, out: &mut ProblemSet) {
    for rel in &dict.relationships {
        let Some(join) = &rel.join else { continue };

        // Skip if any join column references a missing table or column. The
        // missing reference is already reported (S02 / S03), and checking
        // cardinality against a column that doesn't exist would just produce a
        // redundant, confusing S06.
        let all_cols_resolve = join.qcols().all(|q| {
            dict.table(&q.table)
                .is_some_and(|t| t.column(&q.column).is_some())
        });
        if !all_cols_resolve {
            continue;
        }

        // The cardinality rule is defined in terms of the LHS and RHS tables
        // of the join. With multi-conjunct joins (date-range overlap), the
        // LHS and RHS tables are the same across all conjuncts, so we can
        // use the first conjunct as the canonical orientation.
        let Some(first) = join.conjuncts.first() else {
            continue;
        };
        let lhs_table = first.lhs.table.clone();
        let rhs_table = first.rhs.table.clone();

        // Which columns are "the join side" for each table?  For the
        // single-conjunct equality case this is straightforward. For
        // multi-conjunct (range) joins we require ALL conjunct columns on the
        // "one" side to be jointly unique-implied — in practice users
        // typically mark just one of them as PK/unique. We err on the
        // permissive side and check whether *any* column on the "one" side
        // is unique-implied; that matches the loose intuition behind range
        // joins without producing noise for legitimate overlap joins.

        let lhs_cols_unique =
            side_has_unique_implied(dict, &lhs_table, join, /* use_lhs = */ true);
        let rhs_cols_unique =
            side_has_unique_implied(dict, &rhs_table, join, /* use_lhs = */ false);

        let card_span = rel.cardinality.span.clone();
        match rel.cardinality.value {
            Cardinality::OneToOne => {
                if !lhs_cols_unique || !rhs_cols_unique {
                    out.push_spec_error(
                        "S06",
                        "A `one-to-one` join must have `primary_key` or `unique` columns on both sides.",
                        format!(
                            "the join columns on `{}` or `{}` are not marked `primary_key` or `unique`",
                            lhs_table, rhs_table
                        ),
                        [
                            rel.join_text.span.clone(),
                            card_span,
                        ],
                    );
                }
            }
            Cardinality::OneToMany => {
                // Spec: "from left to right" — one row on the left maps to
                // many on the right, so the left side is the "one" side.
                if !lhs_cols_unique {
                    out.push_spec_error(
                        "S06",
                        "A `one-to-many` join must have a `primary_key` or `unique` column on its left (\"one\") side.",
                        format!(
                            "the left-side join column on `{}` is not marked `primary_key` or `unique`",
                            lhs_table
                        ),
                        [
                            rel.join_text.span.clone(),
                            card_span,
                        ],
                    );
                }
            }
            Cardinality::ManyToOne => {
                if !rhs_cols_unique {
                    out.push_spec_error(
                        "S06",
                        "A `many-to-one` join must have a `primary_key` or `unique` column on its right (\"one\") side.",
                        format!(
                            "the right-side join column on `{}` is not marked `primary_key` or `unique`",
                            rhs_table
                        ),
                        [
                            rel.join_text.span.clone(),
                            card_span,
                        ],
                    );
                }
            }
        }
    }
}

fn side_has_unique_implied(
    dict: &DataDict,
    table_name: &str,
    join: &JoinExpr,
    use_lhs: bool,
) -> bool {
    let Some(table) = dict.table(table_name) else {
        return false;
    };
    join.conjuncts.iter().any(|conj| {
        let q: &QCol = if use_lhs { &conj.lhs } else { &conj.rhs };
        if q.table != table_name {
            return false;
        }
        table
            .column(&q.column)
            .is_some_and(|c| c.is_unique_implied())
    })
}

// --- S07 --------------------------------------------------------------

const RANGE_TYPES: &[&str] = &["number(ordinal)", "number(quantity)", "date", "datetime"];

/// Returns whether the column carries the representation its type requires and
/// no other — i.e. whether checking that representation's values (S12) makes
/// sense.
fn validate_s07_representation(table: &Table, col: &Column, out: &mut ProblemSet) -> bool {
    let Some(col_type) = &col.col_type else {
        return true;
    };
    let type_name = col_type.value.as_str();

    let found = |key: &str| format!("has type `{type_name}` but uses `{key}`");
    let missing = |key: &str| format!("has type `{type_name}` but is missing `{key}`");

    // A finding is reported on a specific line — the `type` line for a missing
    // representation, the offending key's line for a present one — with the
    // table and column shown as faded context above it.
    let at = |span: &SourceInfo| [table.name.span.clone(), col.name.span.clone(), span.clone()];

    let before = out.items.len();
    if type_name == "enum" {
        if col.values.is_none() {
            out.push_spec_error(
                "S07",
                "An `enum` column must list its categories with `values`.",
                missing("values"),
                at(&col_type.span),
            );
        }
        if let Some(range) = &col.range {
            out.push_spec_error(
                "S07",
                "An `enum` column must use `values`, not `range`.",
                found("range"),
                at(&range.span),
            );
        }
        if let Some(examples) = &col.examples {
            out.push_spec_error(
                "S07",
                "An `enum` column must use `values`, not `examples`.",
                found("examples"),
                at(&examples.span),
            );
        }
    } else if RANGE_TYPES.contains(&type_name) {
        if col.range.is_none() {
            out.push_spec_error(
                "S07",
                format!("A `{type_name}` column must describe its bounds with `range`."),
                missing("range"),
                at(&col_type.span),
            );
        }
        if let Some(values) = &col.values {
            out.push_spec_error(
                "S07",
                format!("A `{type_name}` column must use `range`, not `values`."),
                found("values"),
                at(values),
            );
        }
        if let Some(examples) = &col.examples {
            out.push_spec_error(
                "S07",
                format!("A `{type_name}` column must use `range`, not `examples`."),
                found("examples"),
                at(&examples.span),
            );
        }
    } else if type_name == "boolean" {
        for (span, key) in [
            (col.values.as_ref(), "values"),
            (col.range.as_ref().map(|r| &r.span), "range"),
            (col.examples.as_ref().map(|e| &e.span), "examples"),
        ] {
            if let Some(span) = span {
                out.push_spec_error(
                    "S07",
                    "A `boolean` column must not have `values`, `range`, or `examples`.",
                    found(key),
                    at(span),
                );
            }
        }
    } else {
        if col.examples.is_none() {
            out.push_spec_error(
                "S07",
                format!("A `{type_name}` column must describe its data with `examples`."),
                missing("examples"),
                at(&col_type.span),
            );
        }
        if let Some(values) = &col.values {
            out.push_spec_error(
                "S07",
                format!("A `{type_name}` column must not use `values`."),
                found("values"),
                at(values),
            );
        }
        if let Some(range) = &col.range {
            out.push_spec_error(
                "S07",
                format!("A `{type_name}` column must not use `range`."),
                found("range"),
                at(&range.span),
            );
        }
    }
    out.items.len() == before
}

// --- S08 --------------------------------------------------------------

fn validate_s08_units(table: &Table, col: &Column, out: &mut ProblemSet) {
    let Some(units) = &col.units else { return };
    let is_quantity = col
        .col_type
        .as_ref()
        .is_some_and(|t| t.value == "number(quantity)");
    if is_quantity {
        return;
    }
    let type_desc = col
        .col_type
        .as_ref()
        .map_or_else(|| "no type".to_string(), |t| format!("type `{}`", t.value));
    out.push_spec_error(
        "S08",
        "A column with `units` must have type `number(quantity)`.",
        format!("has `units` but has {type_desc}"),
        [
            table.name.span.clone(),
            col.name.span.clone(),
            units.span.clone(),
        ],
    );
}

// --- S14 --------------------------------------------------------------

fn validate_s14_time_zone(table: &Table, col: &Column, out: &mut ProblemSet) {
    let Some(time_zone) = &col.time_zone else {
        return;
    };
    let is_datetime = col.col_type.as_ref().is_some_and(|t| t.value == "datetime");
    if is_datetime {
        return;
    }
    let type_desc = col
        .col_type
        .as_ref()
        .map_or_else(|| "no type".to_string(), |t| format!("type `{}`", t.value));
    let mut spans = vec![table.name.span.clone(), col.name.span.clone()];
    if let Some(col_type) = &col.col_type {
        spans.push(col_type.span.clone());
    }
    spans.push(time_zone.span.clone());
    out.push_spec_error(
        "S14",
        "A column with `time_zone` must have type `datetime`.",
        format!("has `time_zone` but has {type_desc}"),
        spans,
    );
}

// --- S15 --------------------------------------------------------------

/// The IANA areas the `Area/Location` form accepts, mirroring the spec's
/// `Time zones` section: the continents and oceans plus `Etc`.
const TIME_ZONE_AREAS: &[&str] = &[
    "Africa",
    "America",
    "Antarctica",
    "Arctic",
    "Asia",
    "Atlantic",
    "Australia",
    "Europe",
    "Indian",
    "Pacific",
    "Etc",
];

/// Whether `tz` has the accepted `time_zone` shape: `naive`, `UTC`, or an
/// `Area/Location` name with a known area. Checks the shape, not the full tzdb,
/// so the accepted set doesn't go stale as zones are added or renamed.
fn time_zone_well_formed(tz: &str) -> bool {
    if tz == "naive" || tz == "UTC" {
        return true;
    }
    let Some((area, location)) = tz.split_once('/') else {
        return false;
    };
    TIME_ZONE_AREAS.contains(&area)
        && !location.is_empty()
        && location
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '/'))
}

fn validate_s15_time_zone_format(table: &Table, col: &Column, out: &mut ProblemSet) {
    let Some(time_zone) = &col.time_zone else {
        return;
    };
    if time_zone_well_formed(&time_zone.value) {
        return;
    }
    let mut spans = vec![table.name.span.clone(), col.name.span.clone()];
    if let Some(col_type) = &col.col_type {
        spans.push(col_type.span.clone());
    }
    spans.push(time_zone.span.clone());
    out.push_spec_error(
        "S15",
        "A `time_zone` must be `naive`, `UTC`, or an IANA `Area/Location` name.",
        "is not a valid time zone",
        spans,
    );
}

// --- S10 --------------------------------------------------------------

fn validate_s10_unique_name(
    table: &Table,
    col: &Column,
    fold_case: bool,
    seen: &mut HashMap<String, SourceInfo>,
    out: &mut ProblemSet,
) {
    let key = s10_key(&col.name.value, fold_case);
    match seen.get(&key) {
        Some(first) => out.push_spec_error(
            "S10",
            s10_column_message(fold_case),
            "is duplicated",
            [
                table.name.span.clone(),
                first.clone(),
                col.name.span.clone(),
            ],
        ),
        None => {
            seen.insert(key, col.name.span.clone());
        }
    }
}

/// the key names are deduplicated under: exact for legacy (parquet is
/// case-sensitive), ASCII-lowercased for rich (duckdb folds identifier case;
/// same folding as `names_eq` in the meta level — duckdb's own Unicode
/// folding is deliberately not replicated)
fn s10_key(name: &str, fold_case: bool) -> String {
    if fold_case {
        name.to_ascii_lowercase()
    } else {
        name.to_string()
    }
}

fn s10_column_message(fold_case: bool) -> &'static str {
    if fold_case {
        // rich collisions may differ in case, so say why they still collide
        "Column names must be unique within a table (DuckDB identifiers are case-insensitive)."
    } else {
        "Column names must be unique within a table."
    }
}

fn validate_s10_unique_table_name(
    table: &Table,
    fold_case: bool,
    seen: &mut HashMap<String, SourceInfo>,
    out: &mut ProblemSet,
) {
    let key = s10_key(&table.name.value, fold_case);
    let message = if fold_case {
        "Table names must be unique within the dictionary (DuckDB identifiers are case-insensitive)."
    } else {
        "Table names must be unique within the dictionary."
    };
    match seen.get(&key) {
        Some(first) => out.push_spec_error(
            "S10",
            message,
            "is duplicated",
            [first.clone(), table.name.span.clone()],
        ),
        None => {
            seen.insert(key, table.name.span.clone());
        }
    }
}

// --- S11 --------------------------------------------------------------

/// Returns whether the table has a name (so its uniqueness may be checked).
fn validate_s11_table_name(table: &Table, out: &mut ProblemSet) -> bool {
    if table.name.value.is_empty() {
        out.push_spec_error(
            "S11",
            "A table must have a non-empty name.",
            "table name is empty",
            [table.name.span.clone()],
        );
        return false;
    }
    true
}

/// Returns whether the column has a name (so its uniqueness may be checked).
fn validate_s11_column_name(table: &Table, col: &Column, out: &mut ProblemSet) -> bool {
    if col.name.value.is_empty() {
        out.push_spec_error(
            "S11",
            "Every column must have a non-empty `name`.",
            "the `name` is empty",
            [table.name.span.clone(), col.name.span.clone()],
        );
        return false;
    }
    true
}

// --- S12 --------------------------------------------------------------

/// The representation list whose values are type-checked for a given column
/// type, or `None` for types that carry no typed representation (`enum`,
/// `boolean`, and any unrecognized type). Mirrors S07: each type owns exactly
/// one representation key, and we only check the one it owns so that a
/// misplaced key reports as S07 rather than cascading into S12.
fn typed_representation(col: &Column) -> Option<(&'static str, &[Spanned<Scalar>])> {
    match col.col_type.as_ref()?.value.as_str() {
        "number(ordinal)" | "number(quantity)" | "date" | "datetime" => {
            Some(("range", &col.range.as_ref()?.items))
        }
        "string" | "number" | "number(id)" => Some(("examples", &col.examples.as_ref()?.items)),
        _ => None,
    }
}

/// Returns whether every value in the column's typed representation matches its
/// type — i.e. whether the bounds are sound enough to compare for order (S13).
fn validate_s12_value_types(table: &Table, col: &Column, out: &mut ProblemSet) -> bool {
    let Some(type_name) = col.col_type.as_ref().map(|t| t.value.as_str()) else {
        return true;
    };
    let Some((key, values)) = typed_representation(col) else {
        return true;
    };
    let tz_present = col.time_zone.is_some();
    let mut ok = true;
    for v in values {
        if value_matches_type(type_name, &v.value, tz_present) {
            continue;
        }
        ok = false;
        out.push_spec_error(
            "S12",
            format!(
                "Each `{}` value of a `{}` column must be {}.",
                key,
                type_name,
                expected_noun(type_name, tz_present),
            ),
            format!("is {}", v.value.noun()),
            [
                table.name.span.clone(),
                col.name.span.clone(),
                v.span.clone(),
            ],
        );
    }
    ok
}

pub(crate) fn is_infinite(value: &Scalar) -> bool {
    matches!(value, Scalar::Number(f) if f.is_infinite())
}

// the rich (duckdb) path has a parallel of this keyed on `TypeCategory`
// (rich::bound_matches_category / range_descending) — keep the two in step
fn value_matches_type(type_name: &str, value: &Scalar, tz_present: bool) -> bool {
    match type_name {
        "number" | "number(id)" | "number(ordinal)" | "number(quantity)" => {
            matches!(value, Scalar::Number(_))
        }
        // The YAML parser discards quote style, so a quoted `'1'` arrives as a
        // number and a quoted `'null'` as null; we can't tell those from a real
        // string. So `string` accepts any scalar and only rejects a list/map.
        "string" => !matches!(value, Scalar::Compound),
        // An infinite bound leaves that end of a temporal range open (spec:
        // Representative values), so accept it alongside a real ISO 8601 value.
        "date" => {
            is_infinite(value) || matches!(value, Scalar::String(s) if parse_date(s).is_some())
        }
        "datetime" => {
            is_infinite(value)
                || matches!(value, Scalar::String(s) if datetime_parses(s, tz_present))
        }
        _ => true,
    }
}

pub(crate) fn parse_date(s: &str) -> Option<NaiveDate> {
    s.parse().ok()
}

pub(crate) fn parse_datetime(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).ok()
}

pub(crate) fn parse_naive_datetime(s: &str) -> Option<NaiveDateTime> {
    s.parse().ok()
}

/// A `datetime` column carries its zone in `time_zone` (`tz_present`), so its
/// values are written zoneless; without one, each value must carry its own
/// offset.
fn datetime_parses(s: &str, tz_present: bool) -> bool {
    if tz_present {
        parse_naive_datetime(s).is_some()
    } else {
        parse_datetime(s).is_some()
    }
}

fn expected_noun(type_name: &str, tz_present: bool) -> &'static str {
    match type_name {
        "string" => "a string",
        "date" => "an ISO 8601 date (YYYY-MM-DD)",
        "datetime" if tz_present => "a zoneless ISO 8601 datetime (e.g. 2024-01-31T09:30:00)",
        "datetime" => "an ISO 8601 datetime with a timezone (e.g. 2024-01-31T09:30:00Z)",
        _ => "a number",
    }
}

// --- S13 --------------------------------------------------------------

/// Only reached when S12 confirmed both bounds parse for the column's type, so
/// `range_descending` can compare them meaningfully.
fn validate_s13_range_order(table: &Table, col: &Column, out: &mut ProblemSet) {
    let Some(type_name) = col.col_type.as_ref().map(|t| t.value.as_str()) else {
        return;
    };
    let Some(range) = &col.range else { return };
    if !RANGE_TYPES.contains(&type_name) || range.items.len() != 2 {
        return;
    }
    let (lo, hi) = (&range.items[0], &range.items[1]);
    if range_descending(type_name, &lo.value, &hi.value, col.time_zone.is_some()) {
        out.push_spec_error(
            "S13",
            "A range's minimum must be less than or equal to its maximum.",
            "is greater than the maximum",
            [
                table.name.span.clone(),
                col.name.span.clone(),
                lo.span.clone(),
            ],
        );
    }
}

/// Whether `lo`..`hi` runs backwards for the column's type. Returns `false`
/// unless both bounds parse as the type's value (a mistyped bound is S12's to
/// report). Numbers compare numerically; dates and datetimes compare as parsed
/// instants, so mixed timezone offsets are handled correctly. A datetime column
/// with a `time_zone` (`tz_present`) has zoneless bounds, compared as wall-clock.
/// An infinite bound orders as `-inf` < any value < `+inf`, so it runs backwards
/// only when it sits on the wrong end (`+inf` as minimum, `-inf` as maximum).
fn range_descending(type_name: &str, lo: &Scalar, hi: &Scalar, tz_present: bool) -> bool {
    if is_infinite(lo) || is_infinite(hi) {
        let is_pos = |v: &Scalar| matches!(v, Scalar::Number(f) if *f == f64::INFINITY);
        let is_neg = |v: &Scalar| matches!(v, Scalar::Number(f) if *f == f64::NEG_INFINITY);
        return (is_pos(lo) && !is_pos(hi)) || (is_neg(hi) && !is_neg(lo));
    }
    match (type_name, lo, hi) {
        ("date", Scalar::String(a), Scalar::String(b)) => match (parse_date(a), parse_date(b)) {
            (Some(a), Some(b)) => a > b,
            _ => false,
        },
        ("datetime", Scalar::String(a), Scalar::String(b)) if tz_present => {
            match (parse_naive_datetime(a), parse_naive_datetime(b)) {
                (Some(a), Some(b)) => a > b,
                _ => false,
            }
        }
        ("datetime", Scalar::String(a), Scalar::String(b)) => {
            match (parse_datetime(a), parse_datetime(b)) {
                (Some(a), Some(b)) => a > b,
                _ => false,
            }
        }
        (_, Scalar::Number(a), Scalar::Number(b)) => a > b,
        _ => false,
    }
}

// --- S16 --------------------------------------------------------------

/// Warn when a single-table dictionary carries `description` or `details` on
/// the table: for one table, those describe the dataset as a whole and belong
/// at the top level.
fn validate_s16_single_table_description(dict: &DataDict, out: &mut ProblemSet) {
    if dict.tables.len() != 1 {
        return;
    }
    let table = dict.tables.first().expect("one table");
    for (key, span) in [
        ("description", &table.description),
        ("details", &table.details),
    ] {
        let Some(span) = span else { continue };
        out.push_spec_warning(
            "S16",
            "A single-table dictionary's description and details belong at the top level.",
            format!("table `{}` has a `{key}`", table.name.value),
            [table.name.span.clone(), span.clone()],
        );
    }
}

// --- S09 --------------------------------------------------------------

/// Warn when the document omits the recommended `$learn_more` key. Unlike the
/// other rules this inspects the raw AST, because `$learn_more` is top-level
/// metadata that the lowered [`DataDict`] does not carry. The warning is
/// anchored at the `$version` key, which the schema guarantees is present.
fn validate_s09_learn_more(root: &YamlWithSourceInfo, out: &mut ProblemSet) {
    let Some(entries) = root.as_hash() else {
        return;
    };
    let has = |key: &str| entries.iter().find(|e| e.key.yaml.as_str() == Some(key));
    if has("$learn_more").is_some() {
        return;
    }
    let span = has("$version")
        .map(|e| e.key_span.clone())
        .unwrap_or_else(|| root.source_info.clone());
    // Insert the recommended key at the very start of the anchor line.
    let insert_at = subspan(&span, 0, 0).unwrap_or_else(|| span.clone());
    out.push_spec_warning(
        "S09",
        "A document should point readers to the spec with `$learn_more`.",
        "`$learn_more` is not set",
        [span],
    );
    out.suggest_last(Suggestion {
        title: "point readers to the spec".into(),
        replacement: format!("$learn_more: {LEARN_MORE_URL}\n"),
        span: insert_at,
    });
}

// --- S17 --------------------------------------------------------------

/// Check the optional top-level `version`. The schema has already fixed its
/// shape (a map whose only keys are `number`, `date`, or `hash`, each with the
/// right value type); S17 enforces the semantic rules the schema can't: a
/// `version` must carry exactly one of those keys, a `number` must have three
/// dot-separated numeric components (with an optional suffix), and a `date` must
/// be a valid ISO 8601 date. Like S09, it reads the raw AST because `version` is
/// top-level metadata the lowered [`DataDict`] does not carry.
fn validate_s17_version(root: &YamlWithSourceInfo, out: &mut ProblemSet) {
    let Some(entries) = root.as_hash() else {
        return;
    };
    let Some(version) = entries
        .iter()
        .find(|e| e.key.yaml.as_str() == Some("version"))
    else {
        return;
    };
    let Some(fields) = version.value.as_hash() else {
        return;
    };

    let expected = "A `version` must give exactly one of `number`, `date`, or `hash`.";
    match fields {
        [] => {
            out.push_spec_error(
                "S17",
                expected,
                "names none of them",
                [version.key_span.clone()],
            );
            return;
        }
        [_] => {}
        [first, .., last] => {
            // The last key is the offending one: a kind was already supplied
            // before it. Highlight it, with the `version` key and the kind it
            // duplicates shown faded above.
            let already = first.key.yaml.as_str().unwrap_or("");
            out.push_spec_error(
                "S17",
                expected,
                format!("`{already}` has already been supplied"),
                [
                    version.key_span.clone(),
                    first.key_span.clone(),
                    last.key_span.clone(),
                ],
            );
            return;
        }
    }

    let field = &fields[0];
    let spans = || [version.key_span.clone(), field.value_span.clone()];
    let text = field.value.yaml.as_str();
    match field.key.yaml.as_str() {
        Some("number") if text.is_none_or(|s| !is_version_number(s)) => {
            out.push_spec_error(
                "S17",
                "A `version` `number` must have three dot-separated numeric components, with an optional pre-release/build suffix.",
                match text {
                    Some(s) => format!("`{s}` is not a valid version number"),
                    None => "is not a valid version number".to_string(),
                },
                spans(),
            );
        }
        Some("date") if text.is_none_or(|s| parse_date(s).is_none()) => {
            out.push_spec_error(
                "S17",
                "A `version` `date` must be an ISO 8601 date (YYYY-MM-DD).",
                match text {
                    Some(s) => format!("`{s}` is not an ISO 8601 date"),
                    None => "is not an ISO 8601 date".to_string(),
                },
                spans(),
            );
        }
        _ => {}
    }
}

/// A version `number` per the spec: three dot-separated numeric components
/// (`MAJOR.MINOR.PATCH`), with an optional semver pre-release (`-…`) and/or
/// build (`+…`) suffix whose dot-separated identifiers are alphanumeric or `-`.
fn is_version_number(s: &str) -> bool {
    let (rest, build) = match s.split_once('+') {
        Some((rest, build)) => (rest, Some(build)),
        None => (s, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };

    let mut parts = core.split('.');
    let numeric = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let core_ok = matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(a), Some(b), Some(c), None) if numeric(a) && numeric(b) && numeric(c)
    );

    let suffix_ok = |s: &str| {
        s.split('.')
            .all(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    };

    core_ok && pre.is_none_or(suffix_ok) && build.is_none_or(suffix_ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_schemas_compile() {
        let _ = legacy_schema();
        let _ = rich_schema();
    }
}
