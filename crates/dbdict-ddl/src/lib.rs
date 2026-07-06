//! DuckDB DDL generator: turn a lowered dictionary into an executable SQL
//! script — `CREATE TYPE` per typedef, then `CREATE TABLE` per table.
//!
//! The generator consumes the model from `dbdict::load_and_lower` only. All
//! DuckDB knowledge (identifier quoting, type-creation order, proving the
//! script executes) comes from `dbdict-duckdb`; this crate is assembly.

use std::fmt;

use dbdict::model::{DataDict, Format, Typedef};
use dbdict_duckdb::{execute_and_describe, quote_ident, typedef_creation_order};

/// One flat-namespace typedef collision: the alias name (as first written)
/// and every place it is defined. `None` is the global `typedef:` block,
/// `Some(table)` a table-scoped block.
#[derive(Debug)]
pub struct Collision {
    pub name: String,
    pub sites: Vec<Option<String>>,
}

/// Why generation refused or failed.
#[derive(Debug)]
pub enum DdlError {
    /// only the rich (0.2.0) format carries DuckDB types to generate from
    LegacyUnsupported,
    /// typedef names that collide in a flat script's single namespace —
    /// table-scoped shadowing works in validation (each table gets its own
    /// scratch database) but cannot be spelled in one script. v1 refuses
    /// rather than inventing a renaming scheme
    Shadowing { collisions: Vec<Collision> },
    /// typedefs duckdb could not create (unknown, cyclic, or malformed),
    /// as `(alias name, duckdb's error)`
    TypedefsStalled { failures: Vec<(String, String)> },
    /// the assembled script failed its scratch-database self-check —
    /// usually a column type duckdb rejects
    ScriptFailed { error: String },
}

impl fmt::Display for DdlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DdlError::LegacyUnsupported => {
                write!(
                    f,
                    "cannot generate DDL from a legacy (0.1.0) dictionary: its \
                     coarse semantic types are not DuckDB types"
                )
            }
            DdlError::Shadowing { collisions } => {
                writeln!(
                    f,
                    "cannot generate a flat script: these typedef names are \
                     defined more than once (DuckDB type names are \
                     case-insensitive and database-global):"
                )?;
                for c in collisions {
                    let sites: Vec<String> = c
                        .sites
                        .iter()
                        .map(|site| match site {
                            None => "the global typedef block".to_string(),
                            Some(table) => format!("table \"{table}\""),
                        })
                        .collect();
                    writeln!(f, "  \"{}\": defined in {}", c.name, sites.join(", "))?;
                }
                write!(f, "rename the colliding typedefs to generate DDL")
            }
            DdlError::TypedefsStalled { failures } => {
                writeln!(f, "these typedefs cannot be created:")?;
                for (name, error) in failures {
                    writeln!(f, "  \"{name}\": {error}")?;
                }
                write!(f, "fix the typedefs (see `dbdict resolve`) to generate DDL")
            }
            DdlError::ScriptFailed { error } => {
                write!(f, "the generated script failed to execute: {error}")
            }
        }
    }
}

impl std::error::Error for DdlError {}

/// Generate an executable DuckDB DDL script from a lowered dictionary:
/// `CREATE TYPE` per typedef (globals and table-scoped alike, in an order
/// discovered by executing them against a scratch database), then
/// `CREATE TABLE` per table in document order.
///
/// Untyped columns make no type claim and are omitted; a table with no typed
/// columns is skipped with a SQL comment saying so. Before returning, the
/// whole script is executed against a fresh in-memory DuckDB, so an `Ok`
/// script is proven runnable.
///
/// Column constraints (`primary_key`, `required`, `unique`) are deliberately
/// **not** emitted as `PRIMARY KEY`/`NOT NULL`/`UNIQUE` clauses — decided
/// 2026-07-06, don't revisit without new evidence. The generated schema's
/// main use is bulk loading, and DuckDB's performance guide is blunt about
/// constraints there: "For best bulk load performance, avoid primary key
/// constraints" (their 554M-row microbenchmark loads ~4x slower with a
/// primary key than without —
/// <https://duckdb.org/docs/current/guides/performance/schema.html>).
/// dbdict's model is declare-then-check: constraints live in the dictionary
/// as declarations, and `validate-data` verifies the loaded data by query
/// (D01 nulls in required/key columns, D02 duplicate primary keys) instead
/// of the database enforcing them row by row during the load.
pub fn generate(dict: &DataDict) -> Result<String, DdlError> {
    if dict.format == Format::Legacy {
        return Err(DdlError::LegacyUnsupported);
    }

    // every typedef — global and table-scoped — lands in the script's one
    // namespace, so collisions must be refused before anything is emitted
    let typedefs = collect_typedefs(dict)?;

    // discover a creation order by executing against a scratch database (the
    // fixpoint trick), instead of parsing type expressions for dependencies
    let order = typedef_creation_order(&typedefs).map_err(|stalled| {
        let failures = stalled
            .into_iter()
            .map(|(index, error)| (typedefs[index].name.value.clone(), error))
            .collect();
        DdlError::TypedefsStalled { failures }
    })?;

    let mut script = String::new();
    for &index in &order {
        let td = typedefs[index];
        script.push_str(&format!(
            "CREATE TYPE {} AS {};\n",
            quote_ident(&td.name.value),
            td.expr.value
        ));
    }

    for table in &dict.tables {
        // a blank line between the types block and each table
        if !script.is_empty() {
            script.push('\n');
        }
        // untyped columns make no type claim; only the typed ones are emitted
        let typed: Vec<(&str, &str)> = table
            .columns
            .iter()
            .filter_map(|c| {
                c.col_type
                    .as_ref()
                    .map(|t| (c.name.value.as_str(), t.value.as_str()))
            })
            .collect();
        if typed.is_empty() {
            // skipped, but not silently — the script says so
            script.push_str(&format!(
                "-- table {} skipped: no typed columns\n",
                quote_ident(&table.name.value)
            ));
            continue;
        }
        script.push_str(&format!(
            "CREATE TABLE {} (\n",
            quote_ident(&table.name.value)
        ));
        let column_lines: Vec<String> = typed
            .iter()
            // the name is an identifier (quoted); the type is an expression,
            // emitted as written — exactly how validation instantiates it
            .map(|(name, col_type)| format!("  {} {}", quote_ident(name), col_type))
            .collect();
        script.push_str(&column_lines.join(",\n"));
        script.push_str("\n);\n");
    }

    // self-check: an Ok script is proven runnable (in a sandboxed scratch
    // database), so a bad column type refuses here instead of failing the user
    execute_and_describe(&script).map_err(|error| DdlError::ScriptFailed { error })?;
    Ok(script)
}

/// Every typedef in the dictionary — the global block first, then each
/// table's scoped ones in document order — after refusing any name collision
/// in the script's single namespace. Names fold ASCII case because DuckDB
/// type names compare case-insensitively.
fn collect_typedefs(dict: &DataDict) -> Result<Vec<&Typedef>, DdlError> {
    // each typedef with the site it was defined at (None = the global block)
    let mut all: Vec<(Option<String>, &Typedef)> = Vec::new();
    for td in &dict.typedefs {
        all.push((None, td));
    }
    for table in &dict.tables {
        for td in &table.typedefs {
            all.push((Some(table.name.value.clone()), td));
        }
    }

    // group by folded name, preserving first-seen order; a Vec scan keeps the
    // report in document order (a HashMap would scramble it) and the counts
    // here are small
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (index, (_, td)) in all.iter().enumerate() {
        let folded = td.name.value.to_ascii_lowercase();
        match groups.iter_mut().find(|(name, _)| *name == folded) {
            Some((_, members)) => members.push(index),
            None => groups.push((folded, vec![index])),
        }
    }
    let collisions: Vec<Collision> = groups
        .iter()
        .filter(|(_, members)| members.len() > 1)
        .map(|(_, members)| Collision {
            name: all[members[0]].1.name.value.clone(),
            sites: members.iter().map(|&i| all[i].0.clone()).collect(),
        })
        .collect();
    if !collisions.is_empty() {
        return Err(DdlError::Shadowing { collisions });
    }
    Ok(all.into_iter().map(|(_, td)| td).collect())
}
