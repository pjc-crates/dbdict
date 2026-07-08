//! Backend-generic dummy-data generation for `dbdict.yaml` dictionaries.
//!
//! This crate turns the lowered model (`dbdict::model::DataDict`) plus
//! generation options into a *plan*: which tables to fill in what order,
//! how many rows each gets, and what each column's values must satisfy
//! (unique key, foreign-key draw, plain fill). It deliberately knows
//! nothing about concrete DuckDB types — rendering a plan into typed SQL
//! literals is `dbdict-dummy-data-duckdb`'s job.

use std::fmt;

use dbdict::model::FkTarget;

mod plan;

pub use plan::{ColumnPlan, GenerateOptions, Plan, RangeBoundKind, Role, TablePlan, plan};

/// Why a dictionary cannot be turned into a generation plan. Every refusal
/// carries enough context to name the offending declaration — mirroring
/// `DdlError`'s style, the message tells the user what to change.
#[derive(Debug)]
pub enum DummyDataError {
    /// dummy data is a rich-format feature; the legacy (parquet) path is
    /// validation-only
    LegacyUnsupported,
    /// tables must be inserted with foreign-key targets first; a dependency
    /// cycle (including a self-referencing fk) has no such order
    ForeignKeyCycle { tables: Vec<String> },
    /// a `foreign_key` column that no relationship pairs with a primary key
    /// — there is nothing to draw values from
    UnresolvedForeignKey { table: String, column: String },
    /// a `foreign_key` column paired with more than one distinct primary
    /// key; a single draw cannot land in all of them
    AmbiguousForeignKey {
        table: String,
        column: String,
        targets: Vec<FkTarget>,
    },
    /// a unique fk column needs one distinct target row per generated row,
    /// but the target table is planned with fewer rows than that
    InjectiveFkExceedsTarget {
        table: String,
        column: String,
        rows: u64,
        target_table: String,
        target_rows: u64,
    },
    /// an fk column must draw values from a target table planned with zero
    /// rows — no valid value exists
    EmptyFkTarget {
        table: String,
        column: String,
        target_table: String,
    },
    /// a per-table row-count override names a table the dictionary does not
    /// declare — almost certainly a typo
    UnknownTableOverride { table: String },
    /// a declared "one" side whose join columns give no uniqueness
    /// guarantee: generated rows could match more than once, violating D05
    CardinalityUnsatisfiable {
        join: String,
        one_table: String,
        columns: Vec<String>,
    },
    /// a join with range operators (`>=`, `<`, …) whose shape the slot
    /// scheme cannot satisfy — `reason` names the offending conjuncts
    RangeJoinUnsupported { join: String, reason: String },
    /// a column a range join must control is also declared `foreign_key`;
    /// slot values and primary-key draws cannot both hold
    RangeColumnIsForeignKey {
        join: String,
        table: String,
        column: String,
    },
    /// a range join can make this column's generated values repeat (rows
    /// sharing a slot owner), but the column is declared `unique` or
    /// `primary_key`
    RangeColumnCannotBeUnique {
        join: String,
        table: String,
        column: String,
    },
    /// two range relationships both claim this column; a column's values
    /// can only satisfy one rule
    RangeColumnConflict { table: String, column: String },
    /// every probe row draws a slot owner on the one side — one *distinct*
    /// owner per row when the draw is injective (one-to-one) — but the one
    /// side is planned with too few rows for that
    RangeProbeExceedsOneSide {
        table: String,
        column: String,
        rows: u64,
        one_table: String,
        one_rows: u64,
    },
    /// the relationship's join string failed to parse (spec check S04);
    /// there is nothing to generate from
    JoinUnparsed { join: String },
    /// the null fraction option must be a proportion in 0.0..=1.0
    NullFractionOutOfRange { value: f64 },
}

impl fmt::Display for DummyDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DummyDataError::LegacyUnsupported => write!(
                f,
                "dummy data can only be generated for rich (0.2.0) dictionaries"
            ),
            DummyDataError::ForeignKeyCycle { tables } => write!(
                f,
                "cannot order tables for insertion: foreign keys form a cycle \
                 through: {}",
                tables.join(", ")
            ),
            DummyDataError::UnresolvedForeignKey { table, column } => write!(
                f,
                "table \"{table}\" column \"{column}\" is declared foreign_key, \
                 but no relationship pairs it with a primary key to draw from"
            ),
            DummyDataError::AmbiguousForeignKey {
                table,
                column,
                targets,
            } => {
                let list: Vec<String> = targets
                    .iter()
                    .map(|t| format!("{}.{}", t.table, t.column))
                    .collect();
                write!(
                    f,
                    "table \"{table}\" column \"{column}\" pairs with more than \
                     one primary key ({}) — one draw cannot satisfy them all",
                    list.join(", ")
                )
            }
            DummyDataError::InjectiveFkExceedsTarget {
                table,
                column,
                rows,
                target_table,
                target_rows,
            } => write!(
                f,
                "table \"{table}\" column \"{column}\" must be unique, but \
                 {rows} rows cannot draw distinct values from the {target_rows} \
                 planned rows of \"{target_table}\" — raise its row count"
            ),
            DummyDataError::EmptyFkTarget {
                table,
                column,
                target_table,
            } => write!(
                f,
                "table \"{table}\" column \"{column}\" draws its values from \
                 \"{target_table}\", which is planned with zero rows"
            ),
            DummyDataError::UnknownTableOverride { table } => write!(
                f,
                "a row-count override names table \"{table}\", which the \
                 dictionary does not declare"
            ),
            DummyDataError::CardinalityUnsatisfiable {
                join,
                one_table,
                columns,
            } => write!(
                f,
                "relationship `{join}` declares \"{one_table}\" as a \"one\" \
                 side, but none of its join columns ({}) is unique or \
                 primary_key, so generated rows could match more than once",
                columns.join(", ")
            ),
            DummyDataError::RangeJoinUnsupported { join, reason } => write!(
                f,
                "relationship `{join}` is outside the supported range-join \
                 shape: {reason}"
            ),
            DummyDataError::RangeColumnIsForeignKey {
                join,
                table,
                column,
            } => write!(
                f,
                "relationship `{join}` needs \"{table}\".\"{column}\" to hold \
                 slot values, but the column is declared foreign_key — it \
                 cannot also draw from a primary key"
            ),
            DummyDataError::RangeColumnCannotBeUnique {
                join,
                table,
                column,
            } => write!(
                f,
                "relationship `{join}` can make \"{table}\".\"{column}\" \
                 repeat values (rows sharing a slot owner), but the column \
                 is declared unique or primary_key"
            ),
            DummyDataError::RangeColumnConflict { table, column } => write!(
                f,
                "column \"{table}\".\"{column}\" is claimed by more than one \
                 range relationship; its values can only satisfy one"
            ),
            DummyDataError::RangeProbeExceedsOneSide {
                table,
                column,
                rows,
                one_table,
                one_rows,
            } => write!(
                f,
                "table \"{table}\" column \"{column}\" lands in the slots of \
                 \"{one_table}\", but {rows} rows cannot draw slot owners \
                 from its {one_rows} planned rows — raise \"{one_table}\"'s \
                 row count"
            ),
            DummyDataError::JoinUnparsed { join } => write!(
                f,
                "relationship join `{join}` does not parse (spec check S04); \
                 fix it before generating data"
            ),
            DummyDataError::NullFractionOutOfRange { value } => write!(
                f,
                "null fraction {value} is not a proportion between 0.0 and 1.0"
            ),
        }
    }
}

impl std::error::Error for DummyDataError {}
