//! The generation-plan builder: dictionary + options → an executable plan.
//!
//! A plan answers three questions the DuckDB half needs before it can emit
//! a single literal: which order to insert tables in (foreign-key targets
//! first), how many rows each table gets, and what each column's values
//! must satisfy — its [`Role`].
//!
//! Everything here is decided from declared constraints alone. The value
//! trick that makes that sound: every generated value is `nth(type, i)`,
//! a pure function of the column type and an index, injective in `i`.
//! So "unique" is satisfied by handing out distinct indices, and "fk
//! matches target pk" is satisfied by both sides computing `nth` at the
//! same index — no data ever needs to be read back.

use std::collections::HashMap;

use dbdict::join_expr::JoinOp;
use dbdict::model::{Cardinality, Constraint, DataDict, FkTarget, Format, Table};

use crate::DummyDataError;

/// Caller-tunable knobs for a generation run.
#[derive(Debug, Clone)]
pub struct GenerateOptions {
    /// rows per table unless overridden below
    pub rows: u64,
    /// per-table row-count overrides, keyed by dictionary table name
    pub table_rows: HashMap<String, u64>,
    /// seed for the plain-fill index choices (used when values are
    /// rendered; the plan itself is seed-independent)
    pub seed: u64,
    /// proportion of rows that get NULL in each optional column,
    /// deterministic per seed; must be within 0.0..=1.0
    pub null_fraction: f64,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            // matches the planned CLI defaults (--rows 10, --seed 0)
            rows: 10,
            table_rows: HashMap::new(),
            seed: 0,
            null_fraction: 0.25,
        }
    }
}

/// What a column's generated values must satisfy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    /// row `i` gets `nth(i)` — distinct by construction (satisfies
    /// `unique` / `primary_key`)
    IndexedUnique,
    /// row `i` gets the target primary-key column's value at some chosen
    /// target row — guaranteed to exist because the target column is
    /// itself index-generated (satisfies `foreign_key`)
    FkDraw {
        target_table: String,
        target_column: String,
        /// each row must pick a *distinct* target row (the fk column is
        /// itself unique, e.g. the many half of a one-to-one)
        injective: bool,
    },
    /// no constraint to satisfy: any index will do, NULLs allowed when
    /// the column plan says so
    PlainFill,
}

/// One column's slice of the plan.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnPlan {
    /// dictionary spelling of the column name
    pub column: String,
    pub role: Role,
    /// whether the null-fraction option may place NULLs here (false for
    /// `required` / `primary_key` columns)
    pub nullable: bool,
}

/// One table's slice of the plan, in insertion order within [`Plan`].
#[derive(Debug, Clone, PartialEq)]
pub struct TablePlan {
    /// dictionary spelling of the table name
    pub table: String,
    pub rows: u64,
    /// columns in dictionary declaration order
    pub columns: Vec<ColumnPlan>,
}

/// The whole generation plan: tables in a safe insertion order — every
/// foreign-key target precedes the tables that draw from it.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub tables: Vec<TablePlan>,
}

/// Build the generation plan for a dictionary, or refuse with a
/// descriptive error naming what to change. Every refusal this crate can
/// see happens here. The one constraint it cannot see is type capacity —
/// the plan is backend-generic and never parses type strings — so "unique
/// column with fewer distinct values than rows" is refused by the backend
/// generator up front, before it renders anything.
pub fn plan(dict: &DataDict, opts: &GenerateOptions) -> Result<Plan, DummyDataError> {
    if dict.format == Format::Legacy {
        return Err(DummyDataError::LegacyUnsupported);
    }
    if !(0.0..=1.0).contains(&opts.null_fraction) {
        return Err(DummyDataError::NullFractionOutOfRange {
            value: opts.null_fraction,
        });
    }
    // catch row-count typos before they silently fall back to the default.
    // hashmap iteration order is random, so sort to keep the reported
    // offender deterministic
    let mut override_names: Vec<&String> = opts.table_rows.keys().collect();
    override_names.sort();
    for name in override_names {
        if dict.table(name).is_none() {
            return Err(DummyDataError::UnknownTableOverride {
                table: name.clone(),
            });
        }
    }

    check_relationships(dict)?;
    let fk_targets = resolve_fk_targets(dict)?;
    let order = insertion_order(dict, &fk_targets)?;

    // small closure so the default-vs-override logic exists exactly once
    let rows_for =
        |table: &str| -> u64 { opts.table_rows.get(table).copied().unwrap_or(opts.rows) };

    let mut tables = Vec::new();
    for t in order {
        let table_name = &t.name.value;
        let rows = rows_for(table_name);
        let mut columns = Vec::new();
        for col in &t.columns {
            let column_name = &col.name.value;
            let role = if col.has(Constraint::ForeignKey) {
                // resolve_fk_targets guaranteed exactly one target per fk
                // column, so this lookup cannot miss
                let target = &fk_targets[&(table_name.clone(), column_name.clone())];
                let target_rows = rows_for(&target.table);
                if rows > 0 && target_rows == 0 {
                    return Err(DummyDataError::EmptyFkTarget {
                        table: table_name.clone(),
                        column: column_name.clone(),
                        target_table: target.table.clone(),
                    });
                }
                let injective = col.is_unique_implied();
                if injective && rows > target_rows {
                    // pigeonhole: more unique draws than distinct targets
                    return Err(DummyDataError::InjectiveFkExceedsTarget {
                        table: table_name.clone(),
                        column: column_name.clone(),
                        rows,
                        target_table: target.table.clone(),
                        target_rows,
                    });
                }
                Role::FkDraw {
                    target_table: target.table.clone(),
                    target_column: target.column.clone(),
                    injective,
                }
            } else if col.is_unique_implied() {
                Role::IndexedUnique
            } else {
                Role::PlainFill
            };
            columns.push(ColumnPlan {
                column: column_name.clone(),
                role,
                nullable: !col.is_required_implied(),
            });
        }
        tables.push(TablePlan {
            table: table_name.clone(),
            rows,
            columns,
        });
    }
    Ok(Plan { tables })
}

/// Refuse relationships the index scheme cannot (yet) satisfy, and verify
/// every declared "one" side actually guarantees at-most-one match.
///
/// D05 counts rows on the probed ("many") side that match more than one
/// row on the other ("one") side. For a pure equality join that count is
/// zero whenever *some* join column on the one side holds distinct
/// values: a probe row can then agree with at most one row there. Roles
/// make declared-unique columns distinct by construction, so the check
/// reduces to "at least one join column on each one side is unique or
/// primary_key" — the same `is_unique_implied` the validator keys off.
fn check_relationships(dict: &DataDict) -> Result<(), DummyDataError> {
    for rel in &dict.relationships {
        let Some(join) = &rel.join else {
            return Err(DummyDataError::JoinUnparsed {
                join: rel.join_text.value.clone(),
            });
        };
        if join.conjuncts.iter().any(|c| c.op != JoinOp::Eq) {
            // range conjuncts need slot-based generation — a later phase
            return Err(DummyDataError::RangeJoinUnsupported {
                join: rel.join_text.value.clone(),
            });
        }
        // orientation comes from the first conjunct, exactly as the D05
        // check reads it (crates/dbdict/src/rich.rs). sides are tracked
        // positionally (left/right booleans, not table names) so a
        // self-join — same table on both sides — still distinguishes them
        let Some(first) = join.conjuncts.first() else {
            continue; // the parser never produces an empty join
        };
        let left_table = &first.lhs.table;
        let right_table = &first.rhs.table;
        let one_side_is_left: &[bool] = match rel.cardinality.value {
            Cardinality::ManyToOne => &[false],
            Cardinality::OneToMany => &[true],
            Cardinality::OneToOne => &[true, false], // "one" both ways
        };
        for &is_left in one_side_is_left {
            let one_table = if is_left { left_table } else { right_table };
            // this side's join columns, canonicalizing each conjunct so
            // its lhs sits on the join's left table (later conjuncts may
            // be written either way round)
            let mut columns = Vec::new();
            for conj in &join.conjuncts {
                let (lhs, rhs) = if conj.lhs.table == *left_table {
                    (&conj.lhs, &conj.rhs)
                } else {
                    (&conj.rhs, &conj.lhs)
                };
                let qcol = if is_left { lhs } else { rhs };
                columns.push(qcol.column.clone());
            }
            // an unknown table or column is simply "not unique" here —
            // spec checks S02/S03 own the real diagnostic
            let satisfied = columns.iter().any(|name| {
                dict.table(one_table)
                    .and_then(|t| t.column(name))
                    .is_some_and(|c| c.is_unique_implied())
            });
            if !satisfied {
                return Err(DummyDataError::CardinalityUnsatisfiable {
                    join: rel.join_text.value.clone(),
                    one_table: one_table.clone(),
                    columns,
                });
            }
        }
    }
    Ok(())
}

/// Resolve every `foreign_key` column to exactly one primary-key target,
/// keyed by `(table, column)` in dictionary spellings.
fn resolve_fk_targets(
    dict: &DataDict,
) -> Result<HashMap<(String, String), FkTarget>, DummyDataError> {
    let mut resolved = HashMap::new();
    for t in &dict.tables {
        for col in &t.columns {
            if !col.has(Constraint::ForeignKey) {
                continue;
            }
            // the same fk→pk pairing declared by two relationships is one
            // target, so dedup before judging ambiguity
            let mut targets: Vec<FkTarget> = Vec::new();
            for target in dict.foreign_key_targets(&t.name.value, &col.name.value) {
                if !targets.contains(&target) {
                    targets.push(target);
                }
            }
            match targets.len() {
                0 => {
                    return Err(DummyDataError::UnresolvedForeignKey {
                        table: t.name.value.clone(),
                        column: col.name.value.clone(),
                    });
                }
                1 => {
                    resolved.insert(
                        (t.name.value.clone(), col.name.value.clone()),
                        targets.pop().expect("len checked"),
                    );
                }
                _ => {
                    return Err(DummyDataError::AmbiguousForeignKey {
                        table: t.name.value.clone(),
                        column: col.name.value.clone(),
                        targets,
                    });
                }
            }
        }
    }
    Ok(resolved)
}

/// Order tables so every foreign-key target comes before the tables that
/// draw from it, or refuse if the dependencies form a cycle.
///
/// This is Kahn's algorithm in its simplest dress: repeatedly place the
/// first (document-order) table whose dependencies are all placed. The
/// repeated scan is O(n²), which is fine at dictionary scale and keeps
/// the tie-break obvious — independent tables keep their document order.
fn insertion_order<'d>(
    dict: &'d DataDict,
    fk_targets: &HashMap<(String, String), FkTarget>,
) -> Result<Vec<&'d Table>, DummyDataError> {
    // deps[i] = names of the tables table i draws values from
    let deps: Vec<Vec<&String>> = dict
        .tables
        .iter()
        .map(|t| {
            t.columns
                .iter()
                .filter_map(|c| fk_targets.get(&(t.name.value.clone(), c.name.value.clone())))
                .map(|target| &target.table)
                .collect()
        })
        .collect();

    let mut order: Vec<&Table> = Vec::new();
    let mut placed = vec![false; dict.tables.len()];
    while order.len() < dict.tables.len() {
        let mut progressed = false;
        for (i, t) in dict.tables.iter().enumerate() {
            if placed[i] {
                continue;
            }
            // a table depending on itself is never ready — it falls
            // through to the cycle error below with the rest of its loop
            let ready = deps[i]
                .iter()
                .all(|dep| order.iter().any(|p| p.name.value == **dep));
            if ready {
                order.push(t);
                placed[i] = true;
                progressed = true;
                break; // restart the scan so earlier tables go first
            }
        }
        if !progressed {
            let tables: Vec<String> = dict
                .tables
                .iter()
                .enumerate()
                .filter(|(i, _)| !placed[*i])
                .map(|(_, t)| t.name.value.clone())
                .collect();
            return Err(DummyDataError::ForeignKeyCycle { tables });
        }
    }
    Ok(order)
}
