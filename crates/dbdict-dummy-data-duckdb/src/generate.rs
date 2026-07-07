//! End-to-end generation: dictionary + options → a SQL script (DDL +
//! INSERTs) and a written `.duckdb` database.
//!
//! The script *is* the deliverable twice over: `write_db` executes it, and
//! the CLI's `--sql` export writes the same text — so what a user debugs is
//! exactly what built their database.
//!
//! Value resolution rides on the plan's roles plus one convention: an
//! *injective* fk draw always picks target row `k = i` (identity). Every
//! fk target is a primary-key column, so its own role is injective too
//! (index-generated or another identity draw), which means "the value
//! stored at target row `k`" can always be *computed* — follow the chain
//! of identity draws down to an index-generated column and call `nth` —
//! without ever reading the database back.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use dbdict::model::DataDict;
use dbdict::rich::InstantiateFailure;
use dbdict_ddl::DdlError;
use dbdict_duckdb::{instantiate, quote_ident};
use dbdict_dummy_data::{DummyDataError, GenerateOptions, Plan, Role};
use duckdb::Connection;

use crate::types::{DuckType, parse_type};
use crate::values::{ValueError, capacity, nth};

/// Why generation (or writing the result) failed.
#[derive(Debug)]
pub enum GenerateError {
    /// the dictionary could not be turned into a plan
    Plan(DummyDataError),
    /// the schema script could not be generated
    Ddl(DdlError),
    /// the dictionary's types do not instantiate on this engine — nothing
    /// can be generated for a column whose type duckdb rejects
    Instantiate { failures: Vec<String> },
    /// a value could not be generated for this column (type exhausted or
    /// outside the supported surface)
    Value {
        table: String,
        column: String,
        error: ValueError,
    },
    /// a unique column's type cannot produce enough distinct values for the
    /// requested rows — refused before any rendering. the plan cannot make
    /// this refusal itself: it is backend-generic and never parses types
    UniqueCapacityTooSmall {
        table: String,
        column: String,
        capacity: u64,
        rows: u64,
    },
    /// `write_db` refuses to touch a path that already exists — the caller
    /// decides about overwriting, never this crate
    OutputExists { path: PathBuf },
    /// the engine rejected something while writing the database
    Db { error: String },
}

impl fmt::Display for GenerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GenerateError::Plan(e) => write!(f, "{e}"),
            GenerateError::Ddl(e) => write!(f, "{e}"),
            GenerateError::Instantiate { failures } => {
                writeln!(f, "the dictionary's types do not instantiate:")?;
                for failure in failures {
                    writeln!(f, "  {failure}")?;
                }
                Ok(())
            }
            GenerateError::Value {
                table,
                column,
                error,
            } => write!(f, "table \"{table}\" column \"{column}\": {error}"),
            GenerateError::UniqueCapacityTooSmall {
                table,
                column,
                capacity,
                rows,
            } => write!(
                f,
                "table \"{table}\" column \"{column}\": {rows} row(s) requested but the \
                 unique column's type can only produce {capacity} distinct value(s) — \
                 lower the row count or widen the type"
            ),
            GenerateError::OutputExists { path } => write!(
                f,
                "output file {} already exists — refusing to overwrite",
                path.display()
            ),
            GenerateError::Db { error } => write!(f, "writing the database failed: {error}"),
        }
    }
}

impl std::error::Error for GenerateError {}

// `?` conversions for the two upstream error types we compose with
impl From<DummyDataError> for GenerateError {
    fn from(e: DummyDataError) -> Self {
        GenerateError::Plan(e)
    }
}

impl From<DdlError> for GenerateError {
    fn from(e: DdlError) -> Self {
        GenerateError::Ddl(e)
    }
}

/// A finished generation: the full SQL script and what it needs to run.
#[derive(Debug, Clone)]
pub struct Generated {
    /// DDL (types + tables) followed by one INSERT per non-empty table,
    /// in foreign-key-safe order — also the `--sql` debug export
    pub script: String,
    /// declared extensions to LOAD before executing the script
    extensions: Vec<String>,
}

impl Generated {
    /// Execute the script into a fresh database file. Refuses a path that
    /// already exists: the dictionary's own database (or anything else)
    /// must never be silently clobbered — deleting first is an explicit
    /// caller decision (the CLI's `--force`).
    pub fn write_db(&self, path: &Path) -> Result<(), GenerateError> {
        if path.exists() {
            return Err(GenerateError::OutputExists {
                path: path.to_path_buf(),
            });
        }
        let conn = Connection::open(path).map_err(|e| GenerateError::Db {
            error: e.to_string(),
        })?;
        for name in &self.extensions {
            // same charset rule as spec check S19, re-checked because the
            // name is interpolated into SQL and dictionaries are untrusted
            let safe = !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            if !safe {
                return Err(GenerateError::Db {
                    error: format!("`{name}` is not a valid extension name"),
                });
            }
            conn.execute(&format!("LOAD {name}"), [])
                .map_err(|e| GenerateError::Db {
                    error: format!("LOAD {name} failed: {e}"),
                })?;
        }
        conn.execute_batch(&self.script)
            .map_err(|e| GenerateError::Db {
                error: e.to_string(),
            })
    }
}

/// Generate dummy data for a dictionary: plan it, render the schema DDL,
/// then append one multi-row INSERT per table with every literal computed
/// by `nth` under the plan's roles. Same dictionary + same options is
/// byte-identical output.
pub fn generate(dict: &DataDict, opts: &GenerateOptions) -> Result<Generated, GenerateError> {
    let plan = dbdict_dummy_data::plan(dict, opts)?;

    // canonical per-column types come from the dictionary's scratch
    // instantiation — `col_type` in the model may be a typedef alias, but
    // DESCRIBE always reports the expanded, canonical spelling `parse_type`
    // was built against
    let inst = instantiate(dict);
    if !inst.failures.is_empty() {
        let failures = inst
            .failures
            .iter()
            .map(|f| render_failure(dict, f))
            .collect();
        return Err(GenerateError::Instantiate { failures });
    }
    // table name → column name → parsed type. untyped columns are absent
    // here *and* absent from the generated DDL, so the INSERTs skip them
    // consistently
    let mut types: HashMap<&str, HashMap<&str, DuckType>> = HashMap::new();
    for (table, cols) in dict.tables.iter().zip(&inst.tables) {
        let parsed = cols
            .iter()
            .map(|(name, canonical)| (name.as_str(), parse_type(canonical)))
            .collect();
        types.insert(table.name.value.as_str(), parsed);
    }

    // capacity refusal up front: a unique column whose type cannot produce
    // one distinct value per row would exhaust mid-render — refuse before
    // rendering anything instead, mirroring the plan's own refusal style.
    // (fk draws need no check here: an injective draw's indices are bounded
    // by the target's rows, which the plan's pigeonhole check already
    // limits, and the target's own unique column is checked in this loop)
    for table_plan in &plan.tables {
        let table_types = &types[table_plan.table.as_str()];
        for column_plan in &table_plan.columns {
            if !matches!(column_plan.role, Role::IndexedUnique) {
                continue;
            }
            // untyped columns are skipped by the DDL and the INSERTs alike
            let Some(ty) = table_types.get(column_plan.column.as_str()) else {
                continue;
            };
            let cap = capacity(ty);
            if cap < table_plan.rows {
                return Err(GenerateError::UniqueCapacityTooSmall {
                    table: table_plan.table.clone(),
                    column: column_plan.column.clone(),
                    capacity: cap,
                    rows: table_plan.rows,
                });
            }
        }
    }

    let mut script = dbdict_ddl::generate(dict)?;
    script.push('\n');
    for table_plan in &plan.tables {
        if table_plan.rows == 0 {
            continue;
        }
        let table_types = &types[table_plan.table.as_str()];
        // only the typed columns exist in the created table
        let columns: Vec<_> = table_plan
            .columns
            .iter()
            .filter(|c| table_types.contains_key(c.column.as_str()))
            .collect();
        if columns.is_empty() {
            continue;
        }
        let column_list: Vec<String> = columns.iter().map(|c| quote_ident(&c.column)).collect();
        script.push_str(&format!(
            "INSERT INTO {} ({}) VALUES\n",
            quote_ident(&table_plan.table),
            column_list.join(", ")
        ));
        for row in 0..table_plan.rows {
            let mut literals = Vec::new();
            for column_plan in &columns {
                literals.push(value_for(
                    &plan,
                    &types,
                    opts,
                    &table_plan.table,
                    column_plan,
                    row,
                )?);
            }
            let terminator = if row + 1 == table_plan.rows { ";" } else { "," };
            script.push_str(&format!("  ({}){}\n", literals.join(", "), terminator));
        }
    }

    Ok(Generated {
        script,
        extensions: dict.extensions.iter().map(|e| e.value.clone()).collect(),
    })
}

type TypeMap<'a> = HashMap<&'a str, HashMap<&'a str, DuckType>>;

/// The literal for one cell, per the column's role. NULL placement runs
/// first: a nullable column goes NULL on a deterministic, seed-dependent
/// subset of rows approximating `null_fraction`.
fn value_for(
    plan: &Plan,
    types: &TypeMap,
    opts: &GenerateOptions,
    table: &str,
    column_plan: &dbdict_dummy_data::ColumnPlan,
    row: u64,
) -> Result<String, GenerateError> {
    let column = column_plan.column.as_str();
    if column_plan.nullable && opts.null_fraction > 0.0 {
        // fraction 1.0 means *every* row: decided exactly, not through the
        // float comparison, which could miss the top-most hash values
        let is_null = opts.null_fraction >= 1.0
            || (mix(opts.seed, &format!("null:{table}.{column}"), row) as f64)
                < opts.null_fraction * u64::MAX as f64;
        if is_null {
            return Ok("NULL".to_string());
        }
    }
    match &column_plan.role {
        Role::IndexedUnique => literal(types, table, column, row),
        Role::FkDraw {
            target_table,
            target_column,
            injective,
        } => {
            let k = if *injective {
                // identity: target row i for source row i. distinct rows
                // pick distinct targets, and chains of unique fk columns
                // all resolve to the same index (see stored_value)
                row
            } else {
                // seed-dependent draw; the plan guaranteed target_rows > 0
                let target_rows = plan
                    .tables
                    .iter()
                    .find(|t| t.table == *target_table)
                    .map(|t| t.rows)
                    .unwrap_or(0)
                    .max(1);
                mix(opts.seed, &format!("fk:{table}.{column}"), row) % target_rows
            };
            stored_value(plan, types, target_table, target_column, k)
        }
        Role::PlainFill => {
            let ty = &types[table][column];
            let cap = capacity(ty);
            if cap == 0 {
                // unsupported type — let nth produce its descriptive error
                return literal(types, table, column, 0);
            }
            let index = mix(opts.seed, &format!("fill:{table}.{column}"), row) % cap;
            literal(types, table, column, index)
        }
    }
}

/// The value actually stored at `row` of a unique column — what an fk draw
/// must reproduce. Every fk target is a primary-key column, so its role is
/// either index-generated (value is `nth(row)`) or itself an injective
/// (identity) draw — follow that chain. The plan refused fk cycles, so the
/// recursion always terminates.
fn stored_value(
    plan: &Plan,
    types: &TypeMap,
    table: &str,
    column: &str,
    row: u64,
) -> Result<String, GenerateError> {
    let column_plan = plan
        .tables
        .iter()
        .find(|t| t.table == table)
        .and_then(|t| t.columns.iter().find(|c| c.column == column));
    match column_plan.map(|c| &c.role) {
        Some(Role::IndexedUnique) => literal(types, table, column, row),
        Some(Role::FkDraw {
            target_table,
            target_column,
            injective: true,
        }) => stored_value(plan, types, target_table, target_column, row),
        // unreachable by construction (fk targets are pk columns), but a
        // descriptive error beats a panic if that invariant ever moves
        _ => Err(GenerateError::Db {
            error: format!("internal: fk target \"{table}\".\"{column}\" is not index-generated"),
        }),
    }
}

/// `nth` with the table/column context attached to any failure.
fn literal(types: &TypeMap, table: &str, column: &str, i: u64) -> Result<String, GenerateError> {
    let ty = &types[table][column];
    nth(ty, i).map_err(|error| GenerateError::Value {
        table: table.to_string(),
        column: column.to_string(),
        error,
    })
}

/// Deterministic 64-bit hash of (seed, salt, i): FNV-1a over the salt,
/// then a splitmix64 finisher. Not cryptographic — just well-scattered and
/// stable across runs and platforms, so seeded output is reproducible.
fn mix(seed: u64, salt: &str, i: u64) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325; // fnv-1a offset basis
    for byte in salt.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x100000001b3); // fnv prime
    }
    let mut z = h ^ seed.wrapping_mul(0x9e3779b97f4a7c15) ^ i.wrapping_add(0x9e3779b97f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

/// One instantiation failure as a human-readable line, mapping the model
/// indices back to dictionary names.
fn render_failure(dict: &DataDict, failure: &InstantiateFailure) -> String {
    match failure {
        InstantiateFailure::Typedef {
            table,
            index,
            error,
        } => {
            let (scope, name) = match table {
                None => (
                    "global".to_string(),
                    dict.typedefs.get(*index).map(|t| t.name.value.clone()),
                ),
                Some(t) => {
                    let table_name = dict
                        .tables
                        .get(*t)
                        .map(|t| t.name.value.as_str())
                        .unwrap_or("?");
                    (
                        format!("table \"{table_name}\""),
                        dict.tables
                            .get(*t)
                            .and_then(|t| t.typedefs.get(*index))
                            .map(|t| t.name.value.clone()),
                    )
                }
            };
            let name = name.unwrap_or_else(|| "?".to_string());
            format!("{scope} typedef \"{name}\": {error}")
        }
        InstantiateFailure::Column {
            table,
            column,
            error,
        } => {
            let table_name = dict
                .tables
                .get(*table)
                .map(|t| t.name.value.as_str())
                .unwrap_or("?");
            let column_name = dict
                .tables
                .get(*table)
                .and_then(|t| t.columns.get(*column))
                .map(|c| c.name.value.as_str())
                .unwrap_or("?");
            format!("table \"{table_name}\" column \"{column_name}\": {error}")
        }
    }
}
