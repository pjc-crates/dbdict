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
    for (index, error) in create_types_fixpoint(&global_conn, &global_refs).stalled {
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

    // a global's failures were already reported by stage 1 (a global can also
    // fail *here* when it compounds on a broken scoped typedef; the scoped
    // report is the root cause, so this echo is dropped)
    let effective = effective_typedefs(table, globals);
    // create_types_fixpoint borrows the typedefs, so collect the refs (no clone)
    let refs: Vec<&Typedef> = effective.iter().map(|(_, td)| *td).collect();
    for (position, error) in create_types_fixpoint(&conn, &refs).stalled {
        if let Some(scoped_index) = effective[position].0 {
            failures.push(InstantiateFailure::Typedef {
                table: Some(table_index),
                index: scoped_index,
                error,
            });
        }
    }

    // probe each typed column on its own: DESCRIBE canonicalizes per column,
    // so the probes' results assemble into the table's expected side, and one
    // bad column type spares its neighbours
    let mut columns = Vec::new();
    for (column_index, column) in table.columns.iter().enumerate() {
        let Some(col_type) = &column.col_type else {
            continue; // untyped: makes no type claim
        };
        match probe_type(&conn, &col_type.value) {
            Ok(canonical) => columns.push((column.name.value.clone(), canonical)),
            Err(error) => failures.push(InstantiateFailure::Column {
                table: table_index,
                column: column_index,
                error,
            }),
        }
    }
    columns
}

/// A table's effective typedefs: the globals not shadowed by a same-named
/// table-scoped typedef, then the table's own. Each entry is tagged with
/// `Some(scoped_index)` (its position in `table.typedefs`) when it is
/// table-scoped, or `None` for an unshadowed global.
fn effective_typedefs<'a>(
    table: &'a Table,
    globals: &'a [Typedef],
) -> Vec<(Option<usize>, &'a Typedef)> {
    let shadowed = |name: &str| -> bool { table.typedefs.iter().any(|td| td.name.value == name) };
    let mut effective: Vec<(Option<usize>, &'a Typedef)> = Vec::new();
    for td in globals {
        if !shadowed(&td.name.value) {
            effective.push((None, td));
        }
    }
    for (scoped_index, td) in table.typedefs.iter().enumerate() {
        effective.push((Some(scoped_index), td));
    }
    effective
}

/// Create a single-column `probe` table with the given type expression and
/// DESCRIBE it back: duckdb's canonical spelling of that type. Exactly one
/// DESCRIBE row is required — more means the expression smuggled extra
/// columns past us (a top-level comma). This guard is a tripwire, not a
/// boundary: a crafted multi-*statement* expression can still shape its own
/// probe result — but that only lets a dictionary lie about its own expected
/// side, which it controls anyway; the real safety basis is `open_scratch`
/// (throwaway in-memory database, external access off).
fn probe_type(conn: &Connection, type_expr: &str) -> Result<String, String> {
    let sql = format!("CREATE OR REPLACE TABLE probe (x {type_expr})");
    conn.execute(&sql, []).map_err(|e| e.to_string())?;
    let described = describe(conn, "probe")?;
    // slice patterns make "exactly one" explicit: match a one-element slice,
    // binding its fields, and reject every other shape
    match described.as_slice() {
        [(_, canonical)] => Ok(canonical.clone()),
        _ => Err("type expression must describe a single column".to_string()),
    }
}

/// One typedef's canonical expansion, as the `resolve` CLI command prints it.
#[derive(Debug, Clone)]
pub struct TypedefExpansion {
    /// `None` for a global typedef. `Some(table)` for a table-scoped typedef —
    /// and also for a *global* whose expansion changes inside that table
    /// (because a scoped typedef shadows one of its dependencies)
    pub table: Option<String>,
    /// the alias name, as written in the dictionary
    pub name: String,
    /// the declared type expression, as written
    pub expr: String,
    /// duckdb's canonical spelling, or duckdb's error when the alias is
    /// unknown, cyclic, or malformed
    pub expansion: Result<String, String>,
}

/// Expand every typedef to its canonical duckdb spelling: the globals first,
/// then per table anything whose expansion is specific to that table, all in
/// document order. Same scratch mechanics as [`instantiate`] — create the
/// types by fixpoint, then probe each alias as a single-column table and
/// DESCRIBE the canonical type back.
///
/// A table's entries are its scoped typedefs plus any global whose expansion
/// *differs* in the table's context — a scoped typedef can shadow a
/// dependency of a global (`a: intish` globally, table redefines `intish`),
/// and validation instantiates that table with the reshaped `a`, so the
/// output must say so too or it would contradict `validate-meta`.
pub fn expand_typedefs(dict: &DataDict) -> Vec<TypedefExpansion> {
    let mut out = Vec::new();

    let conn = open_scratch();
    let global_refs: Vec<&Typedef> = dict.typedefs.iter().collect();
    let stalled = create_types_fixpoint(&conn, &global_refs).stalled;
    for (position, td) in dict.typedefs.iter().enumerate() {
        out.push(TypedefExpansion {
            table: None,
            name: td.name.value.clone(),
            expr: td.expr.value.clone(),
            expansion: expansion_result(&conn, td, &stalled, position),
        });
    }

    for table in &dict.tables {
        if table.typedefs.is_empty() {
            continue; // effective typedefs == the globals: nothing table-specific
        }
        // fresh connection with the table's effective typedefs, so everything
        // expands in *this* table's context — exactly how instantiation sees it
        let conn = open_scratch();
        let effective = effective_typedefs(table, &dict.typedefs);
        let refs: Vec<&Typedef> = effective.iter().map(|(_, td)| *td).collect();
        let stalled = create_types_fixpoint(&conn, &refs).stalled;
        for (position, (scoped, td)) in effective.iter().enumerate() {
            let expansion = expansion_result(&conn, td, &stalled, position);
            // an unshadowed global is an echo of the global pass unless this
            // table's scoped typedefs changed what it expands to
            if scoped.is_none() {
                let same_as_global = out.iter().any(|e| {
                    e.table.is_none() && e.name == td.name.value && e.expansion == expansion
                });
                if same_as_global {
                    continue;
                }
            }
            out.push(TypedefExpansion {
                table: Some(table.name.value.clone()),
                name: td.name.value.clone(),
                expr: td.expr.value.clone(),
                expansion,
            });
        }
    }
    out
}

/// One typedef's expansion outcome: the fixpoint's error if it stalled,
/// otherwise the canonical type probed from its alias.
///
/// `position` is this typedef's index in the exact slice handed to
/// [`create_types_fixpoint`] — stalled entries are keyed by that position, so
/// both must come from the same enumerate over the same slice.
fn expansion_result(
    conn: &Connection,
    td: &Typedef,
    stalled: &[(usize, String)],
    position: usize,
) -> Result<String, String> {
    match stalled.iter().find(|(i, _)| *i == position) {
        Some((_, error)) => Err(error.clone()),
        // the alias name is quoted like any identifier, so an odd name is
        // never read as SQL syntax in type position
        None => probe_type(conn, &quote_ident(&td.name.value)),
    }
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

    fn count_nulls(&self, db_file: &Path, table: &str, column: &str) -> Result<usize, String> {
        count_nulls(db_file, table, column)
    }

    fn count_duplicate_keys(
        &self,
        db_file: &Path,
        table: &str,
        key_columns: &[String],
    ) -> Result<usize, String> {
        count_duplicate_keys(db_file, table, key_columns)
    }

    fn count_duplicate_values(
        &self,
        db_file: &Path,
        table: &str,
        column: &str,
    ) -> Result<usize, String> {
        count_duplicate_values(db_file, table, column)
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

/// What the `CREATE TYPE` fixpoint produced: which typedefs were created (in
/// the order their statements succeeded — a valid order for a flat script to
/// replay) and which stalled.
struct FixpointOutcome {
    /// indices into the input slice, in creation order
    created: Vec<usize>,
    /// the stalled leftovers — the cyclic/unknown group — as
    /// `(index, duckdb error)`
    stalled: Vec<(usize, String)>,
}

/// Try to `CREATE TYPE` every typedef, retrying the rejects until a full pass
/// makes no progress.
fn create_types_fixpoint(conn: &Connection, typedefs: &[&Typedef]) -> FixpointOutcome {
    let mut created = Vec::new();
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
                quote_ident(&td.name.value),
                td.expr.value
            );
            match conn.execute(&sql, []) {
                Ok(_) => created.push(index),
                Err(e) => failed.push((index, e.to_string())),
            }
        }
        // done (nothing failed) or stalled (a full round made no progress):
        // either way the leftovers are final
        if failed.is_empty() || failed.len() == pending.len() {
            return FixpointOutcome {
                created,
                stalled: failed,
            };
        }
        pending = failed.into_iter().map(|(index, _)| index).collect();
    }
}

/// The order in which a flat script can `CREATE TYPE` the given typedefs,
/// discovered by executing them against a scratch database (the fixpoint
/// above) rather than by parsing type expressions for dependencies.
///
/// `Ok` holds indices into `typedefs` in a creation order that succeeds;
/// `Err` holds the stalled leftovers (unknown, cyclic, or malformed) as
/// `(index, duckdb error)`.
pub fn typedef_creation_order(typedefs: &[&Typedef]) -> Result<Vec<usize>, Vec<(usize, String)>> {
    let conn = open_scratch();
    let outcome = create_types_fixpoint(&conn, typedefs);
    if outcome.stalled.is_empty() {
        Ok(outcome.created)
    } else {
        Err(outcome.stalled)
    }
}

/// Execute a multi-statement SQL script in a fresh scratch database and
/// DESCRIBE every relation it created, alphabetically by name: proof that a
/// generated script is executable, plus what it builds. Runs with external
/// access disabled (see [`open_scratch`]), so a hostile script cannot touch
/// the filesystem or network.
pub fn execute_and_describe(script: &str) -> Result<Vec<TableSchema>, String> {
    let conn = open_scratch();
    conn.execute_batch(script).map_err(|e| e.to_string())?;
    describe_all(&conn)
}

/// Read every relation (tables *and* views — a dictionary table may
/// legitimately be backed by a view) in the duckdb database at `db_file`,
/// alphabetically by name, each with its canonical column types.
///
/// The database is opened read-only, so it is never created, mutated, or
/// locked for writing.
pub fn read_schema(db_file: &Path) -> Result<Vec<TableSchema>, String> {
    let conn = open_read_only(db_file)?;
    describe_all(&conn)
}

/// Every relation in the connection's database, alphabetically, each with its
/// canonical column types. Shared by [`read_schema`] (the real database) and
/// [`execute_and_describe`] (a scratch database a script just built).
fn describe_all(conn: &Connection) -> Result<Vec<TableSchema>, String> {
    let mut schema = Vec::new();
    for name in relation_names(conn)? {
        schema.push(TableSchema {
            columns: describe(conn, &name)?,
            name,
        });
    }
    Ok(schema)
}

/// Open the database at `db_file` read-only, so it is never created, mutated,
/// or locked for writing.
fn open_read_only(db_file: &Path) -> Result<Connection, String> {
    let config = Config::default()
        .access_mode(AccessMode::ReadOnly)
        .map_err(|e| e.to_string())?;
    Connection::open_with_flags(db_file, config).map_err(|e| e.to_string())
}

/// D01 — how many rows of `table` are null in `column`. Opens its own
/// read-only connection per call, like everything else here (the backend is
/// a unit struct; all state is per-call).
pub fn count_nulls(db_file: &Path, table: &str, column: &str) -> Result<usize, String> {
    let conn = open_read_only(db_file)?;
    let sql = format!(
        "SELECT count(*) FROM {} WHERE {} IS NULL",
        quote_ident(table),
        quote_ident(column)
    );
    query_count(&conn, &sql)
}

/// D02 — how many distinct values of `key_columns` (one composite key) occur
/// in more than one row of `table`. `GROUP BY` treats NULL keys as equal, so
/// repeated all-NULL keys count as duplicates too — D01 flags the nulls
/// themselves separately.
pub fn count_duplicate_keys(
    db_file: &Path,
    table: &str,
    key_columns: &[String],
) -> Result<usize, String> {
    let conn = open_read_only(db_file)?;
    let keys = key_columns
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT count(*) FROM (SELECT 1 FROM {} GROUP BY {} HAVING count(*) > 1)",
        quote_ident(table),
        keys
    );
    query_count(&conn, &sql)
}

/// D03 — how many distinct non-NULL values of `column` occur in more than
/// one row of `table`. NULLs are excluded before grouping: SQL `UNIQUE`
/// semantics treat NULLs as distinct, so an optional-but-unique column may
/// legitimately repeat them (contrast [`count_duplicate_keys`], where a
/// primary key implies `required` and NULL keys are still counted).
pub fn count_duplicate_values(db_file: &Path, table: &str, column: &str) -> Result<usize, String> {
    let conn = open_read_only(db_file)?;
    let col = quote_ident(column);
    let sql = format!(
        "SELECT count(*) FROM (SELECT 1 FROM {} WHERE {col} IS NOT NULL GROUP BY {col} HAVING count(*) > 1)",
        quote_ident(table)
    );
    query_count(&conn, &sql)
}

/// Run a `SELECT count(*)`-shaped query and return its single value.
fn query_count(conn: &Connection, sql: &str) -> Result<usize, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let count = stmt
        .query_row([], |row| row.get::<_, i64>(0))
        .map_err(|e| e.to_string())?;
    // count(*) is never negative; the cast is safe
    Ok(count as usize)
}

/// Double-quote a DuckDB identifier, escaping embedded quotes, so a name (or
/// a typedef alias used as a type) is never parsed as SQL syntax. Public
/// because the DDL generator spells identifiers the same way.
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
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
        .prepare(&format!("DESCRIBE {}", quote_ident(table)))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_and_escapes_identifiers() {
        assert_eq!(quote_ident("food"), "\"food\"");
        // an embedded double-quote is doubled
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
    }
}
