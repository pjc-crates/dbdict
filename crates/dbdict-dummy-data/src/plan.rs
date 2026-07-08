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

use dbdict::join_expr::{JoinExpr, JoinOp};
use dbdict::model::{Cardinality, Constraint, DataDict, FkTarget, Format, Relationship, Table};

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

/// Which edge of a slot a one-side bound column holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeBoundKind {
    Lower,
    Upper,
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
    /// one side of a range join: row `i`'s value is a slot edge —
    /// `nth(3i)` for the lower bound, `nth(3i + 2)` for the upper.
    /// stride 3 leaves `nth(3i + 1)` strictly between the edges, so open
    /// (`>`/`<`) and closed (`>=`/`<=`) bounds are satisfied alike, and
    /// monotonicity of `nth` keeps the slots disjoint
    RangeBound {
        /// index into `dict.relationships` — ties the column to its join
        rel: usize,
        kind: RangeBoundKind,
    },
    /// probe ("many") side of a range join: the row picks a slot owner
    /// `k` on the one side and takes `nth(3k + 1)` — inside slot `k` and
    /// outside every other slot, so it matches at most one row (D05)
    RangeProbe {
        /// index into `dict.relationships` — the backend salts the owner
        /// draw with this, so all of a row's roles for one relationship
        /// agree on `k`
        rel: usize,
        /// the side that owns the slots (the declared "one" side)
        one_table: String,
        /// each probe row must pick a *distinct* owner (one-to-one)
        injective: bool,
    },
    /// equality conjunct riding along a range join: the probe row copies
    /// the slot owner's value in this column, so the eq conjunct agrees
    /// with the range conjuncts about which row matches
    SlotEqCopy {
        /// index into `dict.relationships` — same salt as the probe, so
        /// the copy reads the same owner `k`
        rel: usize,
        one_table: String,
        one_column: String,
        /// mirror of the probe's draw: identity owners on one-to-one.
        /// the copy must agree with the probe about `k`, so the flag
        /// travels with the role instead of being re-derived from the
        /// cardinality by the backend
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

impl Plan {
    /// The row count assigned to `table` in this plan, or 0 when no such
    /// table is planned. The backend looks this up per fk/range draw to
    /// bound its owner index — one method so those call sites cannot
    /// disagree about how a missing table is treated
    pub fn planned_rows(&self, table: &str) -> u64 {
        self.tables
            .iter()
            .find(|t| t.table == table)
            .map(|t| t.rows)
            .unwrap_or(0)
    }
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

    let range_claims = check_relationships(dict)?;
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
            // a range-relationship claim overrides the constraint-derived
            // role: the relationship dictates this column's values
            let claim = range_claims.get(&(table_name.clone(), column_name.clone()));
            let role = if let Some(claim) = claim {
                claim.clone()
            } else if col.has(Constraint::ForeignKey) {
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
            // range probes and slot copies both draw a slot owner on the
            // one side: there must be owners to draw at all, and enough
            // distinct ones when the draw is injective (one-to-one)
            // both range-draw roles now carry the same two fields, so one
            // or-pattern arm covers them (they share the identical draw)
            let owner_draw = match &role {
                Role::RangeProbe {
                    one_table,
                    injective,
                    ..
                }
                | Role::SlotEqCopy {
                    one_table,
                    injective,
                    ..
                } => Some((one_table.clone(), *injective)),
                _ => None,
            };
            if let Some((one_table, injective)) = owner_draw {
                let one_rows = rows_for(&one_table);
                if (rows > 0 && one_rows == 0) || (injective && rows > one_rows) {
                    return Err(DummyDataError::RangeProbeExceedsOneSide {
                        table: table_name.clone(),
                        column: column_name.clone(),
                        rows,
                        one_table,
                        one_rows,
                    });
                }
            }
            // range-role columns must always hold a value — the slots (and
            // the probe values inside them) have to exist for the join to
            // land — so they ignore the null-fraction option entirely
            let nullable = match role {
                Role::RangeBound { .. } | Role::RangeProbe { .. } | Role::SlotEqCopy { .. } => {
                    false
                }
                _ => !col.is_required_implied(),
            };
            columns.push(ColumnPlan {
                column: column_name.clone(),
                role,
                nullable,
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

/// Check every relationship is satisfiable by construction, and collect
/// the column roles that range joins impose.
///
/// D05 counts rows on the probed ("many") side that match more than one
/// row on the other ("one") side. Two ways to guarantee a zero count:
///
/// * **pure equality join** — some join column on the one side holds
///   distinct values: a probe row can then agree with at most one row
///   there. Roles make declared-unique columns distinct by construction,
///   so the check reduces to "at least one join column on each one side
///   is unique or primary_key" — the same `is_unique_implied` the
///   validator keys off.
/// * **range join** (any `>=`/`>`/`<=`/`<` conjunct) — the one side's
///   bound columns hold disjoint slots and every probe value lands
///   strictly inside exactly one slot, so matches are at most one
///   without any uniqueness requirement. These relationships *claim*
///   their columns: the returned map assigns each one a range [`Role`]
///   that overrides its constraint-derived role.
fn check_relationships(dict: &DataDict) -> Result<HashMap<(String, String), Role>, DummyDataError> {
    let mut claims: HashMap<(String, String), Role> = HashMap::new();
    for (rel_index, rel) in dict.relationships.iter().enumerate() {
        let Some(join) = &rel.join else {
            return Err(DummyDataError::JoinUnparsed {
                join: rel.join_text.value.clone(),
            });
        };
        if join.conjuncts.is_empty() {
            continue; // the parser never produces an empty join
        }
        if join.conjuncts.iter().any(|c| c.op != JoinOp::Eq) {
            claim_range_roles(dict, rel_index, rel, join, &mut claims)?;
            continue;
        }
        // orientation is owned by JoinExpr::oriented — the same helper the
        // D05 validator reads (crates/dbdict/src/rich.rs), so planner and
        // validator cannot disagree about which side is "one". probing a
        // direction makes the *other* side the one side
        for &probe_left in rel.cardinality.value.probe_left_directions() {
            let (_, one_table) = join.sides(probe_left);
            // the one side's join columns: the `other` end of each
            // oriented conjunct
            let columns: Vec<String> = join
                .oriented(probe_left)
                .iter()
                .map(|oc| oc.other.column.clone())
                .collect();
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
                    one_table: one_table.to_string(),
                    columns,
                });
            }
        }
    }
    Ok(claims)
}

/// Assign slot roles for a relationship with range conjuncts, or refuse.
///
/// A range join is direction-sensitive: the "one" side owns the slots (its
/// two bound columns) and the probe side holds the single bounded value.
/// [`Cardinality::probe_left_directions`] gives the directions D05 checks —
/// one for many-to-one/one-to-many (writing order fixes which side is
/// "many"), but *both* for one-to-one, which is symmetric. So we try each
/// candidate direction and keep the first that yields a valid slot shape;
/// only if every direction fails do we surface a refusal (the first
/// direction's, since that is the one the writing order suggests).
fn claim_range_roles(
    dict: &DataDict,
    rel_index: usize,
    rel: &Relationship,
    join: &JoinExpr,
    claims: &mut HashMap<(String, String), Role>,
) -> Result<(), DummyDataError> {
    let injective = rel.cardinality.value == Cardinality::OneToOne;
    let mut first_err: Option<DummyDataError> = None;
    for &probe_left in rel.cardinality.value.probe_left_directions() {
        // build this direction's claims in a scratch map, so a failed
        // attempt never pollutes `claims` for the next direction
        let mut attempt: HashMap<(String, String), Role> = HashMap::new();
        match claim_range_direction(
            dict,
            rel_index,
            rel,
            join,
            probe_left,
            injective,
            &mut attempt,
        ) {
            Ok(()) => {
                // merge into the real map, conflict-checked across relationships
                for ((table, column), role) in attempt {
                    claim(claims, &table, &column, role)?;
                }
                return Ok(());
            }
            // remember the first direction's error — it matches the reading
            // order, so the message points where the author likely looked
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    Err(first_err.expect("probe_left_directions is never empty"))
}

/// Try to claim slot roles for one probe direction, writing into `attempt`.
/// Returns an error (leaving `attempt` partially filled, which the caller
/// discards) when the conjuncts do not form a valid slot shape.
fn claim_range_direction(
    dict: &DataDict,
    rel_index: usize,
    rel: &Relationship,
    join: &JoinExpr,
    probe_left: bool,
    injective: bool,
    claims: &mut HashMap<(String, String), Role>,
) -> Result<(), DummyDataError> {
    let (probe_table, one_table) = join.sides(probe_left);

    // dictionary lookups for the constraint checks; a column the
    // dictionary does not declare simply has no constraints to conflict
    // with — unknown names are S02/S03's diagnostics, not ours
    let lookup = |table: &str, column: &str| dict.table(table).and_then(|t| t.column(column));

    // bucket the conjuncts by the probe column they constrain, keeping
    // first-appearance order so refusals point at a deterministic column
    struct ProbeBucket<'j> {
        probe_column: &'j str,
        /// one-side columns bounding the probe from below (`>=`/`>`)
        lower: Vec<&'j str>,
        /// one-side columns bounding the probe from above (`<=`/`<`)
        upper: Vec<&'j str>,
        /// one-side columns the probe must equal
        eq: Vec<&'j str>,
    }
    let mut buckets: Vec<ProbeBucket> = Vec::new();
    for oc in join.oriented(probe_left) {
        // find-or-append by probe column. done via position + index
        // because holding the `find` result borrow would block the push
        // (a borrow-checker limit, not a logic constraint)
        let at = match buckets
            .iter()
            .position(|b| b.probe_column == oc.probe.column)
        {
            Some(at) => at,
            None => {
                buckets.push(ProbeBucket {
                    probe_column: &oc.probe.column,
                    lower: Vec::new(),
                    upper: Vec::new(),
                    eq: Vec::new(),
                });
                buckets.len() - 1
            }
        };
        let bucket = &mut buckets[at];
        match oc.op {
            JoinOp::Eq => bucket.eq.push(&oc.other.column),
            JoinOp::Ge | JoinOp::Gt => bucket.lower.push(&oc.other.column),
            JoinOp::Le | JoinOp::Lt => bucket.upper.push(&oc.other.column),
        }
    }

    // one shorthand so every shape refusal below stays a one-liner
    let refuse = |reason: String| -> Result<(), DummyDataError> {
        Err(DummyDataError::RangeJoinUnsupported {
            join: rel.join_text.value.clone(),
            reason,
        })
    };
    for bucket in &buckets {
        if !bucket.eq.is_empty() {
            // a probe column that is both range-bounded and copied, or
            // copied from two different owner columns, is outside the
            // scheme — one column cannot satisfy both value rules
            if !bucket.lower.is_empty() || !bucket.upper.is_empty() {
                return refuse(format!(
                    "column \"{}\" appears in both range and equality conjuncts",
                    bucket.probe_column
                ));
            }
            if bucket.eq.len() != 1 {
                return refuse(format!(
                    "column \"{}\" is set equal to more than one column of \"{}\"",
                    bucket.probe_column, one_table
                ));
            }
            let probe_col = lookup(probe_table, bucket.probe_column);
            if probe_col.is_some_and(|c| c.has(Constraint::ForeignKey)) {
                return Err(DummyDataError::RangeColumnIsForeignKey {
                    join: rel.join_text.value.clone(),
                    table: probe_table.to_string(),
                    column: bucket.probe_column.to_string(),
                });
            }
            // copies repeat whenever two rows share an owner, and even
            // injective owners only yield distinct copies when the source
            // column's values are themselves distinct
            let source_unique =
                lookup(one_table, bucket.eq[0]).is_some_and(|c| c.is_unique_implied());
            if probe_col.is_some_and(|c| c.is_unique_implied()) && !(injective && source_unique) {
                return Err(DummyDataError::RangeColumnCannotBeUnique {
                    join: rel.join_text.value.clone(),
                    table: probe_table.to_string(),
                    column: bucket.probe_column.to_string(),
                });
            }
            claim(
                claims,
                probe_table,
                bucket.probe_column,
                Role::SlotEqCopy {
                    rel: rel_index,
                    one_table: one_table.to_string(),
                    one_column: bucket.eq[0].to_string(),
                    injective,
                },
            )?;
            continue;
        }
        // the supported range shape: one lower + one upper bound, held in
        // two distinct one-side columns
        if bucket.lower.len() != 1 {
            return refuse(format!(
                "column \"{}\" needs exactly one lower bound (`>=`/`>`), found {}",
                bucket.probe_column,
                bucket.lower.len()
            ));
        }
        if bucket.upper.len() != 1 {
            return refuse(format!(
                "column \"{}\" needs exactly one upper bound (`<=`/`<`), found {}",
                bucket.probe_column,
                bucket.upper.len()
            ));
        }
        if bucket.lower[0] == bucket.upper[0] {
            return refuse(format!(
                "column \"{}\" must be bounded by two distinct columns of \
                 \"{}\", not \"{}\" twice",
                bucket.probe_column, one_table, bucket.lower[0]
            ));
        }
        // none of the three claimed columns may also be a foreign key —
        // slot values and primary-key draws cannot both hold
        for (t, c) in [
            (probe_table, bucket.probe_column),
            (one_table, bucket.lower[0]),
            (one_table, bucket.upper[0]),
        ] {
            if lookup(t, c).is_some_and(|col| col.has(Constraint::ForeignKey)) {
                return Err(DummyDataError::RangeColumnIsForeignKey {
                    join: rel.join_text.value.clone(),
                    table: t.to_string(),
                    column: c.to_string(),
                });
            }
        }
        // a non-injective draw repeats slot owners, so probe values repeat
        // too. bound columns need no such check: their values are slot
        // edges, injective in the row index by construction
        if !injective
            && lookup(probe_table, bucket.probe_column).is_some_and(|c| c.is_unique_implied())
        {
            return Err(DummyDataError::RangeColumnCannotBeUnique {
                join: rel.join_text.value.clone(),
                table: probe_table.to_string(),
                column: bucket.probe_column.to_string(),
            });
        }
        claim(
            claims,
            one_table,
            bucket.lower[0],
            Role::RangeBound {
                rel: rel_index,
                kind: RangeBoundKind::Lower,
            },
        )?;
        claim(
            claims,
            one_table,
            bucket.upper[0],
            Role::RangeBound {
                rel: rel_index,
                kind: RangeBoundKind::Upper,
            },
        )?;
        claim(
            claims,
            probe_table,
            bucket.probe_column,
            Role::RangeProbe {
                rel: rel_index,
                one_table: one_table.to_string(),
                injective,
            },
        )?;
    }
    Ok(())
}

/// Record one range claim, refusing when a different claim already owns
/// the column. Re-stating the identical claim is harmless (one
/// relationship may use the same bound column for two probe columns).
fn claim(
    claims: &mut HashMap<(String, String), Role>,
    table: &str,
    column: &str,
    role: Role,
) -> Result<(), DummyDataError> {
    let key = (table.to_string(), column.to_string());
    match claims.get(&key) {
        None => {
            claims.insert(key, role);
            Ok(())
        }
        Some(existing) if *existing == role => Ok(()),
        Some(_) => Err(DummyDataError::RangeColumnConflict {
            table: table.to_string(),
            column: column.to_string(),
        }),
    }
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
