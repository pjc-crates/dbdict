//! Native (in-process) duckdb access, built on the bundled `duckdb` crate.
//!
//! This is the real side of the rich round-trip: open the database the
//! dictionary's `source` names and read back every relation with its
//! canonical column types. Errors cross the seam as plain strings — duckdb's
//! own messages — which the core maps to located problems.

use std::path::Path;

use dbdict::model::{DataDict, Table, Typedef};
use dbdict::rich::{InstantiateFailure, Instantiated, TableSchema, TypeCategory};
use duckdb::{AccessMode, Config, Connection};

/// Instantiate the dictionary in scratch in-memory databases and DESCRIBE
/// what was created: the *expected* side of the round-trip.
///
/// Each table gets its own fresh connection holding its *effective* typedefs
/// (the globals, with same-named table-scoped ones shadowing them) — `CREATE
/// TYPE` names are database-global, so shadowing cannot live in one shared
/// scratch database. Typedefs resolve by fixpoint (retry until nothing new
/// succeeds), so declaration order never matters and the stalled leftovers
/// are exactly the cyclic/unknown group, reported with duckdb's own error.
/// Each typed column is probed as its own single-column table, so one bad
/// column type spares its neighbours' expected types.
///
/// Panics only if an in-memory duckdb cannot be created at all (effectively
/// resource exhaustion) — there is no dictionary input that causes this.
pub fn instantiate(dict: &DataDict) -> Instantiated {
    let mut failures = Vec::new();

    // stage 1: the global typedefs alone, so a broken global reports once
    // (not once per table) and at its own span
    let global_conn = open_scratch();
    let global_refs: Vec<&Typedef> = dict.typedefs.iter().collect();
    for (index, error) in create_types_fixpoint(&global_conn, &global_refs) {
        failures.push(InstantiateFailure::Typedef {
            table: None,
            index,
            error,
        });
    }

    let mut tables = Vec::new();
    for (table_index, table) in dict.tables.iter().enumerate() {
        tables.push(instantiate_table(
            table_index,
            table,
            &dict.typedefs,
            &mut failures,
        ));
    }

    Instantiated { tables, failures }
}

/// Build one table's scratch database and return its typed columns' canonical
/// types, pushing this table's own failures (scoped typedefs, bad column
/// types) onto `failures`.
fn instantiate_table(
    table_index: usize,
    table: &Table,
    globals: &[Typedef],
    failures: &mut Vec<InstantiateFailure>,
) -> Vec<(String, String)> {
    let conn = open_scratch();

    // effective typedefs: globals not shadowed by a table-scoped name, then
    // the table's own. each is tagged with `Some(scoped_index)` when it is a
    // table-scoped typedef, or `None` for an unshadowed global — a global's
    // failures were already reported by stage 1 (a global can also fail *here*
    // when it compounds on a broken scoped typedef; the scoped report is the
    // root cause, so this echo is dropped)
    let shadowed = |name: &str| -> bool { table.typedefs.iter().any(|td| td.name.value == name) };
    let mut effective: Vec<(Option<usize>, &Typedef)> = Vec::new();
    for td in globals {
        if !shadowed(&td.name.value) {
            effective.push((None, td));
        }
    }
    for (scoped_index, td) in table.typedefs.iter().enumerate() {
        effective.push((Some(scoped_index), td));
    }
    // create_types_fixpoint borrows the typedefs, so collect the refs (no clone)
    let refs: Vec<&Typedef> = effective.iter().map(|(_, td)| *td).collect();
    for (position, error) in create_types_fixpoint(&conn, &refs) {
        if let Some(scoped_index) = effective[position].0 {
            failures.push(InstantiateFailure::Typedef {
                table: Some(table_index),
                index: scoped_index,
                error,
            });
        }
    }

    // probe each typed column as a single-column table: DESCRIBE canonicalizes
    // per column, so the probes' rows assemble into the table's expected side
    let mut columns = Vec::new();
    for (column_index, column) in table.columns.iter().enumerate() {
        let Some(col_type) = &column.col_type else {
            continue; // untyped: makes no type claim
        };
        let mut fail = |error: String| {
            failures.push(InstantiateFailure::Column {
                table: table_index,
                column: column_index,
                error,
            });
        };
        let sql = format!(
            "CREATE OR REPLACE TABLE probe ({} {})",
            crate::quote_ident(&column.name.value),
            col_type.value
        );
        if let Err(e) = conn.execute(&sql, []) {
            fail(e.to_string());
            continue;
        }
        match describe(&conn, "probe") {
            // exactly one row is the norm; more than one means the type
            // expression smuggled extra columns (a top-level comma), so its
            // canonical form is not trustworthy — reject rather than let the
            // phantom columns leak into the expected side
            Ok(described) if described.len() == 1 => columns.extend(described),
            Ok(_) => fail("type expression must describe a single column".to_string()),
            Err(error) => fail(error),
        }
    }
    columns
}

/// The native backend: what the CLI hands to `dbdict::validate_meta`. A unit
/// struct because all state (connections, scratch databases) is per-call.
pub struct NativeDuckdb;

impl dbdict::rich::DuckdbBackend for NativeDuckdb {
    fn instantiate(&self, dict: &DataDict) -> Instantiated {
        instantiate(dict)
    }

    fn read_schema(&self, db_file: &Path) -> Result<Vec<TableSchema>, String> {
        read_schema(db_file)
    }

    fn classify(&self, canonical_type: &str) -> TypeCategory {
        classify(canonical_type)
    }
}

/// Classify a canonical type spelling (as `DESCRIBE` returns it) for the
/// descriptive-key checks. Composite shapes are checked first: `FLOAT[768]`
/// starts with a numeric word but is an array, so a bracket anywhere in the
/// spelling means "not a scalar" and classifies as `Other`.
pub fn classify(canonical_type: &str) -> TypeCategory {
    if canonical_type.contains('[') {
        return TypeCategory::Other; // array or list of anything
    }
    match canonical_type {
        "BOOLEAN" => TypeCategory::Boolean,
        "DATE" => TypeCategory::Date,
        // zoneless timestamps at every precision duckdb canonicalizes to
        "TIMESTAMP" | "TIMESTAMP_S" | "TIMESTAMP_MS" | "TIMESTAMP_NS" => TypeCategory::Timestamp,
        "TIMESTAMP WITH TIME ZONE" => TypeCategory::TimestampTz,
        "TINYINT" | "SMALLINT" | "INTEGER" | "BIGINT" | "HUGEINT" | "UTINYINT" | "USMALLINT"
        | "UINTEGER" | "UBIGINT" | "UHUGEINT" | "FLOAT" | "DOUBLE" => TypeCategory::Numeric,
        t if t.starts_with("DECIMAL(") => TypeCategory::Numeric,
        t if t.starts_with("ENUM(") => TypeCategory::Enum,
        _ => TypeCategory::Other,
    }
}

/// A fresh in-memory scratch database with external access disabled.
///
/// A dictionary is untrusted shared input, and its typedef/column type text is
/// interpolated into DDL. duckdb's `execute` runs *all* statements in a string
/// (not just the first), so a type expression can smuggle extra statements;
/// `enable_external_access(false)` stops `ATTACH`/`COPY`/`read_csv` and the
/// like from touching the filesystem or network. Combined with the database
/// being a throwaway in-memory instance (and the real database opened
/// read-only), a hostile dictionary can at worst make instantiation fail.
fn open_scratch() -> Connection {
    let config = Config::default()
        .enable_external_access(false)
        .expect("static config value always applies");
    Connection::open_in_memory_with_flags(config)
        .expect("failed to create the in-memory scratch database")
}

/// Try to `CREATE TYPE` every typedef, retrying the rejects until a full pass
/// makes no progress. Returns `(index, duckdb error)` for the stalled
/// leftovers — the cyclic/unknown group — where `index` is the typedef's
/// position in `typedefs`.
fn create_types_fixpoint(conn: &Connection, typedefs: &[&Typedef]) -> Vec<(usize, String)> {
    let mut pending: Vec<usize> = (0..typedefs.len()).collect();
    loop {
        // each round carries its own error alongside the index, so there is no
        // parallel array to keep in step and a stalled index always has a real
        // message (never a silent default)
        let mut failed: Vec<(usize, String)> = Vec::new();
        for &index in &pending {
            let td = typedefs[index];
            let sql = format!(
                "CREATE TYPE {} AS {}",
                crate::quote_ident(&td.name.value),
                td.expr.value
            );
            if let Err(e) = conn.execute(&sql, []) {
                failed.push((index, e.to_string()));
            }
        }
        // done (nothing failed) or stalled (a full round made no progress):
        // either way the leftovers are final
        if failed.is_empty() || failed.len() == pending.len() {
            return failed;
        }
        pending = failed.into_iter().map(|(index, _)| index).collect();
    }
}

/// Read every relation (tables *and* views — a dictionary table may
/// legitimately be backed by a view) in the duckdb database at `db_file`,
/// alphabetically by name, each with its canonical column types.
///
/// The database is opened read-only, so it is never created, mutated, or
/// locked for writing.
pub fn read_schema(db_file: &Path) -> Result<Vec<TableSchema>, String> {
    let config = Config::default()
        .access_mode(AccessMode::ReadOnly)
        .map_err(|e| e.to_string())?;
    let conn = Connection::open_with_flags(db_file, config).map_err(|e| e.to_string())?;
    let mut schema = Vec::new();
    for name in relation_names(&conn)? {
        schema.push(TableSchema {
            columns: describe(&conn, &name)?,
            name,
        });
    }
    Ok(schema)
}

/// Every user relation in the database's `main` schema, alphabetically.
/// `information_schema.tables` covers base tables and views but not duckdb's
/// internal catalogs.
fn relation_names(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'main' ORDER BY table_name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// `DESCRIBE` one relation: its `(column, canonical type)` pairs in table
/// order. Shared by the real side here and the scratch side.
pub(crate) fn describe(conn: &Connection, table: &str) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(&format!("DESCRIBE {}", crate::quote_ident(table)))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
