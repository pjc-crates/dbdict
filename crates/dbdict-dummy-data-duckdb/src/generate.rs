//! End-to-end generation: dictionary + options → a SQL script (DDL +
//! INSERTs) and a written `.duckdb` database.
//!
//! The script *is* the deliverable twice over: `write_db` executes it, and
//! the CLI's `--sql` export writes the same text — so what a user debugs is
//! exactly what built their database. Declared duckdb extensions are folded
//! in as leading `LOAD` statements, so the script is self-contained: running
//! it on a bare connection reproduces the database with no other setup.
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
use dbdict_dummy_data::{DummyDataError, GenerateOptions, Plan, RangeBoundKind, Role};
use duckdb::Connection;

use crate::types::{DuckType, parse_type};
use crate::values::{ValueError, capacity, is_orderable, nth};

/// Range-join slot width: each "one"-side row owns `SLOT_STRIDE` consecutive
/// `nth` indices — a lower edge, an interior probe point, and an upper edge.
/// Stride 3 is the *minimum* that leaves an index strictly between the edges
/// (`nth(3i)` < `nth(3i + 1)` < `nth(3i + 2)`), so a probe value can sit inside
/// slot `i` and outside every other slot for open and closed bounds alike; a
/// stride of 2 would leave no interior index. `nth` is monotone for orderable
/// types, so distinct slots never overlap. Roles are defined in
/// `dbdict-dummy-data`'s `plan.rs`; this is where they turn into indices.
const SLOT_STRIDE: u64 = 3;

/// A duckdb extension name safe to interpolate into a `LOAD` statement:
/// non-empty, lowercase ASCII letters / digits / underscores. This is spec
/// check S19's rule, re-applied here because `generate` is a public entry
/// point that can be handed a dictionary which never went through spec
/// validation, and an extension name becomes part of the emitted SQL.
fn is_safe_extension_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

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
    /// a range-join column's *type* cannot serve the slot scheme (not
    /// orderable, or disagreeing with the join's other columns) — the
    /// structural shapes were already refused by the plan
    RangeUnsupported {
        table: String,
        column: String,
        reason: String,
    },
    /// a range join needs 3 slot values per one-side row; this type
    /// cannot produce that many distinct values
    RangeCapacityTooSmall {
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
            GenerateError::RangeUnsupported {
                table,
                column,
                reason,
            } => write!(
                f,
                "table \"{table}\" column \"{column}\": range join unsupported — {reason}"
            ),
            GenerateError::RangeCapacityTooSmall {
                table,
                column,
                capacity,
                rows,
            } => write!(
                f,
                "table \"{table}\" column \"{column}\": {rows} one-side row(s) need \
                 {} slot values but the type can only produce {capacity} — lower the \
                 row count or widen the type",
                rows.saturating_mul(SLOT_STRIDE)
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

/// A finished generation: the full, self-contained SQL script.
#[derive(Debug, Clone)]
pub struct Generated {
    /// leading `LOAD` per declared extension, then the DDL (types + tables),
    /// then one INSERT per non-empty table in foreign-key-safe order — a
    /// complete, standalone script, and exactly what the `--sql` export writes
    pub script: String,
}

impl Generated {
    /// Execute the script into a fresh database file. Refuses a path that
    /// already exists: the dictionary's own database (or anything else)
    /// must never be silently clobbered — deleting first is an explicit
    /// caller decision (the CLI's `--force`).
    ///
    /// The script self-contains its extension `LOAD`s, so executing it is the
    /// whole job — no separate setup, and running the `--sql` export by hand
    /// does exactly the same thing.
    pub fn write_db(&self, path: &Path) -> Result<(), GenerateError> {
        if path.exists() {
            return Err(GenerateError::OutputExists {
                path: path.to_path_buf(),
            });
        }
        let conn = Connection::open(path).map_err(|e| GenerateError::Db {
            error: e.to_string(),
        })?;
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
    // consistently. the canonical DESCRIBE spellings are kept alongside:
    // the range checks compare and report types by that exact string
    let mut types: HashMap<&str, HashMap<&str, DuckType>> = HashMap::new();
    let mut canonical: CanonMap = HashMap::new();
    for (table, cols) in dict.tables.iter().zip(&inst.tables) {
        let parsed = cols
            .iter()
            .map(|(name, canonical)| (name.as_str(), parse_type(canonical)))
            .collect();
        types.insert(table.name.value.as_str(), parsed);
        let spelled = cols
            .iter()
            .map(|(name, canonical)| (name.as_str(), canonical.as_str()))
            .collect();
        canonical.insert(table.name.value.as_str(), spelled);
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

    // range joins get the same up-front treatment: refuse type-level
    // shapes the slot scheme cannot serve before rendering anything
    check_range_types(&plan, &types, &canonical)?;

    // declared extensions LOAD first, so any type or table that depends on one
    // (e.g. a JSON column) resolves when the script runs. folding the LOADs in
    // here — rather than issuing them separately at write time — is what makes
    // the script, and the `--sql` export of it, a complete standalone reproduction
    let mut script = String::new();
    for ext in &dict.extensions {
        let name = &ext.value;
        if !is_safe_extension_name(name) {
            return Err(GenerateError::Db {
                error: format!("`{name}` is not a valid extension name"),
            });
        }
        script.push_str(&format!("LOAD {name};\n"));
    }
    if !dict.extensions.is_empty() {
        script.push('\n');
    }
    script.push_str(&dbdict_ddl::generate(dict)?);
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

    Ok(Generated { script })
}

type TypeMap<'a> = HashMap<&'a str, HashMap<&'a str, DuckType>>;
/// table name → column name → canonical DESCRIBE spelling of the type
type CanonMap<'a> = HashMap<&'a str, HashMap<&'a str, &'a str>>;

/// Refuse range joins whose *types* cannot serve the slot scheme, before
/// any rendering — the structural shapes were already refused by the
/// plan, so what is left to check here is exactly what the plan cannot
/// see (it never parses type strings):
///
/// * bounds and probes must be orderable — slot containment leans on
///   `nth` being monotone, injectivity alone is not enough
/// * every bound and probe of one relationship must share a canonical
///   type: index arithmetic across two different `nth` sequences proves
///   nothing about containment
/// * slots use indices up to `3·one_rows − 1`, so the type must hold
///   `3·one_rows` distinct values (the range twin of the unique check)
/// * an eq copy renders its *source's* literal, so both columns must
///   share a type, the source must have one at all, and the source's role
///   must be one [`stored_value`] can reproduce by index
/// * every bound and probe column must have a declared type — an untyped
///   one is dropped from the DDL, so its slot values would silently vanish
fn check_range_types(
    plan: &Plan,
    types: &TypeMap,
    canonical: &CanonMap,
) -> Result<(), GenerateError> {
    // first bound/probe seen per relationship — later ones must match it
    let mut rel_first: HashMap<usize, (String, String, String)> = HashMap::new();
    for table_plan in &plan.tables {
        for column_plan in &table_plan.columns {
            let table = table_plan.table.as_str();
            let column = column_plan.column.as_str();
            // eq copies are judged against their source, not the range type
            if let Role::SlotEqCopy {
                one_table,
                one_column,
                ..
            } = &column_plan.role
            {
                let Some(copy_type) = canonical.get(table).and_then(|t| t.get(column)) else {
                    // an untyped copy column is dropped from the DDL, so the
                    // eq conjunct would reference a column that does not exist
                    return Err(GenerateError::RangeUnsupported {
                        table: table.to_string(),
                        column: column.to_string(),
                        reason: "has no declared type but is an equality-copy column of a \
                                 range join — give it a type or drop the conjunct"
                            .to_string(),
                    });
                };
                let source_type = canonical
                    .get(one_table.as_str())
                    .and_then(|t| t.get(one_column.as_str()));
                match source_type {
                    None => {
                        return Err(GenerateError::RangeUnsupported {
                            table: table.to_string(),
                            column: column.to_string(),
                            reason: format!(
                                "its equality source \"{one_table}\".\"{one_column}\" has \
                                 no type — there is no stored value to copy"
                            ),
                        });
                    }
                    Some(source_type) if source_type != copy_type => {
                        return Err(GenerateError::RangeUnsupported {
                            table: table.to_string(),
                            column: column.to_string(),
                            reason: format!(
                                "its type {copy_type} differs from its equality source \
                                 \"{one_table}\".\"{one_column}\" ({source_type}) — a copy \
                                 must share its source's type"
                            ),
                        });
                    }
                    _ => {}
                }
                // the copy reproduces the source's value by index, which only
                // works for roles `stored_value` can recompute (index-unique,
                // an injective fk draw, or plain fill). a source that is itself
                // a seed-dependent draw (non-injective fk) or another slot
                // value cannot be reproduced — refuse rather than emit an
                // internal error mid-generation
                let source_role = plan
                    .tables
                    .iter()
                    .find(|t| t.table == *one_table)
                    .and_then(|t| t.columns.iter().find(|c| c.column == *one_column))
                    .map(|c| &c.role);
                if !source_role.is_some_and(is_recomputable_role) {
                    return Err(GenerateError::RangeUnsupported {
                        table: table.to_string(),
                        column: column.to_string(),
                        reason: format!(
                            "its equality source \"{one_table}\".\"{one_column}\" has a value \
                             that cannot be copied by index (it is a foreign-key draw or a \
                             slot value) — copy a plain, unique, or primary-key column instead"
                        ),
                    });
                }
                continue;
            }
            // bounds and probes: orderable, one shared type, slot capacity
            let (rel, slot_rows) = match &column_plan.role {
                Role::RangeBound { rel, .. } => (*rel, table_plan.rows),
                Role::RangeProbe { rel, one_table, .. } => (*rel, plan.planned_rows(one_table)),
                _ => continue,
            };
            let Some(ty) = types.get(table).and_then(|t| t.get(column)) else {
                // a bound/probe with no type is dropped from the DDL, so its
                // slot values would silently vanish and the join would
                // reference a missing column — refuse cleanly instead
                return Err(GenerateError::RangeUnsupported {
                    table: table.to_string(),
                    column: column.to_string(),
                    reason: "has no declared type but is a range-join bound or probe \
                             column — give it an orderable type"
                        .to_string(),
                });
            };
            let spelled = canonical[table][column];
            if !is_orderable(ty) {
                return Err(GenerateError::RangeUnsupported {
                    table: table.to_string(),
                    column: column.to_string(),
                    reason: format!(
                        "type {spelled} is not orderable — slot bounds and probes \
                         need monotone value generation"
                    ),
                });
            }
            match rel_first.get(&rel) {
                None => {
                    rel_first.insert(
                        rel,
                        (table.to_string(), column.to_string(), spelled.to_string()),
                    );
                }
                Some((first_table, first_column, first_type)) if first_type != spelled => {
                    return Err(GenerateError::RangeUnsupported {
                        table: table.to_string(),
                        column: column.to_string(),
                        reason: format!(
                            "its type {spelled} differs from \"{first_table}\".\
                             \"{first_column}\" ({first_type}) — every bound and probe \
                             of one range join must share a canonical type"
                        ),
                    });
                }
                Some(_) => {}
            }
            let cap = capacity(ty);
            match slot_rows.checked_mul(SLOT_STRIDE) {
                Some(needed) if needed <= cap => {}
                // overflow means the row count is astronomically beyond any
                // type's capacity — same refusal
                _ => {
                    return Err(GenerateError::RangeCapacityTooSmall {
                        table: table.to_string(),
                        column: column.to_string(),
                        capacity: cap,
                        rows: slot_rows,
                    });
                }
            }
        }
    }
    Ok(())
}

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
                // seed-dependent draw; the plan guaranteed target_rows > 0,
                // so max(1) is only a divide-by-zero guard
                let target_rows = plan.planned_rows(target_table).max(1);
                mix(opts.seed, &format!("fk:{table}.{column}"), row) % target_rows
            };
            stored_value(plan, types, opts, target_table, target_column, k)
        }
        Role::PlainFill => plain_fill_value(types, opts, table, column, row),
        Role::RangeBound { kind, .. } => {
            // one-side row i owns slot i: edges at the slot's first and last
            // index (offsets 0 and SLOT_STRIDE-1 == 2). nth is monotone for
            // orderable types, so slots never overlap
            let offset = match kind {
                RangeBoundKind::Lower => 0,
                RangeBoundKind::Upper => 2,
            };
            literal(types, table, column, SLOT_STRIDE * row + offset)
        }
        Role::RangeProbe {
            rel,
            one_table,
            injective,
        } => {
            // the interior index (offset 1) sits strictly between slot k's
            // edges — inside slot k for open and closed bounds alike, and
            // outside every other slot
            let k = owner_for(plan, opts, *rel, one_table, *injective, row);
            literal(types, table, column, SLOT_STRIDE * k + 1)
        }
        Role::SlotEqCopy {
            rel,
            one_table,
            one_column,
            injective,
        } => {
            // same rel-salted draw as the probe, so the eq conjunct and the
            // range conjuncts agree about which one-side row this row matches
            let k = owner_for(plan, opts, *rel, one_table, *injective, row);
            stored_value(plan, types, opts, one_table, one_column, k)
        }
    }
}

/// The slot owner drawn by probe-side row `row` for relationship `rel`.
/// Salted by the relationship *index* — never the column name — so the
/// range probe and every eq copy of one relationship read the same `k`:
/// their values must all point at the same one-side row.
fn owner_for(
    plan: &Plan,
    opts: &GenerateOptions,
    rel: usize,
    one_table: &str,
    injective: bool,
    row: u64,
) -> u64 {
    if injective {
        // one-to-one: owner k = row i, mirroring the injective fk draw —
        // distinct rows own distinct slots by construction
        return row;
    }
    // the plan refused probing a zero-row one side, so `max(1)` is only a
    // belt-and-braces guard against dividing by zero
    let one_rows = plan.planned_rows(one_table).max(1);
    mix(opts.seed, &format!("range:{rel}"), row) % one_rows
}

/// The deterministic plain-fill literal for one cell — shared by
/// `value_for` and `stored_value` so an eq copy recomputes exactly the
/// literal its source row rendered (same salt, same formula).
fn plain_fill_value(
    types: &TypeMap,
    opts: &GenerateOptions,
    table: &str,
    column: &str,
    row: u64,
) -> Result<String, GenerateError> {
    let ty = &types[table][column];
    let cap = capacity(ty);
    if cap == 0 {
        // unsupported type — let nth produce its descriptive error
        return literal(types, table, column, 0);
    }
    let index = mix(opts.seed, &format!("fill:{table}.{column}"), row) % cap;
    literal(types, table, column, index)
}

/// Whether [`stored_value`] can reproduce a column's value from its index
/// alone. True for index-generated columns, injective (identity) fk draws,
/// and plain fill — the cases `stored_value` handles. A non-injective fk
/// draw or a slot value depends on a seed-scattered owner index that a
/// copy cannot reconstruct, so those are false and refused up front.
fn is_recomputable_role(role: &Role) -> bool {
    matches!(
        role,
        Role::IndexedUnique
            | Role::FkDraw {
                injective: true,
                ..
            }
            | Role::PlainFill
    )
}

/// The value another cell must reproduce: what row `row` of this column
/// rendered. Fk draws resolve their target's primary-key value; slot eq
/// copies resolve their source column. Index-generated columns are
/// `nth(row)`, injective (identity) fk draws follow the chain — the plan
/// refused fk cycles, so the recursion terminates — and plain-fill
/// sources recompute the same deterministic fill.
///
/// One caveat, deliberate: a *nullable* plain-fill source may actually
/// have stored NULL (the null-fraction rule runs before roles). The copy
/// still renders the non-NULL fill literal, so the eq conjunct simply
/// matches nothing for that row — zero matches is within D05's "at most
/// one", and it keeps NULLs out of copy columns that may be `required`.
fn stored_value(
    plan: &Plan,
    types: &TypeMap,
    opts: &GenerateOptions,
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
        }) => stored_value(plan, types, opts, target_table, target_column, row),
        Some(Role::PlainFill) => plain_fill_value(types, opts, table, column, row),
        // anything else (non-injective draws, range roles) cannot be
        // recomputed without reading the database — a descriptive error
        // beats a panic if the plan's invariants ever move
        _ => Err(GenerateError::Db {
            error: format!(
                "internal: the stored value of \"{table}\".\"{column}\" \
                 cannot be recomputed"
            ),
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
