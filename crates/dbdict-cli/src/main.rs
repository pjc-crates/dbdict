use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use dbdict::ProblemSet;
use dbdict::model::DataDict;

#[derive(Parser)]
#[command(name = "dbdict", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a dbdict.yaml file or directory against the spec [default: .]
    ValidateSpec { path: Option<PathBuf> },
    /// Validate a dataset's column names and types against a data dictionary
    ValidateMeta(ValidateArgs),
    /// Validate a dataset's values against a data dictionary
    ValidateData(ValidateArgs),
    /// Print each typedef's canonical DuckDB expansion [default: .]
    Resolve { dict: Option<PathBuf> },
    /// Print executable DuckDB DDL generated from a data dictionary [default: .]
    Ddl { dict: Option<PathBuf> },
    /// Generate a DuckDB database of dummy data from a data dictionary [default: .]
    Dummy(DummyArgs),
    /// Print the dbdict.yaml specification
    Spec,
    /// Inspect data types of a data source
    Types {
        #[command(subcommand)]
        command: TypesCommand,
    },
    /// Agents: read these skills to learn how to work with dbdict files
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
}

/// Shared arguments for `validate-meta` and `validate-data`.
#[derive(clap::Args)]
struct ValidateArgs {
    dict: PathBuf,
    /// Validate only this table, instead of every table in the dictionary
    #[arg(long)]
    table: Option<String>,
    /// Emit results as JSON
    #[arg(long)]
    json: bool,
}

/// Arguments for `dummy`. `--out` is required and never defaults to the
/// dictionary's own source file — generating over a real database would be too
/// easy to do by accident.
#[derive(clap::Args)]
struct DummyArgs {
    dict: Option<PathBuf>,
    /// Where to write the generated .duckdb database (required)
    #[arg(short, long)]
    out: PathBuf,
    /// Also write the generated SQL script (DDL + INSERTs) to this file
    #[arg(long)]
    sql: Option<PathBuf>,
    /// Overwrite --out if it already exists (otherwise refuse)
    #[arg(long)]
    force: bool,
    /// Rows per table unless overridden with --rows-table
    #[arg(long, default_value_t = 10)]
    rows: u64,
    /// Per-table row-count override, e.g. --rows-table trades=100 (repeatable)
    #[arg(long = "rows-table", value_name = "TABLE=N")]
    table_rows: Vec<String>,
    /// Seed for reproducible generation
    #[arg(long, default_value_t = 0)]
    seed: u64,
    /// Fraction of each optional column's values to make NULL, 0.0..=1.0
    /// (0.0 fills every value; required and key columns are never nulled)
    #[arg(long, default_value_t = 0.10)]
    null_fraction: f64,
}

#[derive(Subcommand)]
enum SkillCommand {
    /// Skill for reading and understanding a data dictionary
    Read,
    /// Skill for creating or updating a data dictionary
    Write,
}

const READ_SKILL: &str = include_str!("../skills/read-data-dict.md");
const WRITE_SKILL: &str = include_str!("../skills/write-data-dict.md");

#[derive(Subcommand)]
enum TypesCommand {
    /// Print column types for a parquet file
    Parquet { path: PathBuf },
    /// Print every table's column types from a DuckDB database
    Duckdb { path: PathBuf },
}

/// Rust's runtime sets SIGPIPE to SIG_IGN before `main`, so writing to a
/// closed pipe (`dbdict spec | head`) surfaces as an EPIPE error that makes
/// `println!` panic. Restoring SIG_DFL means the process instead dies quietly
/// with SIGPIPE, like other unix CLIs producing textual output.
///
/// `unsafe`: `libc::signal` is a raw C call; safe here because it runs first
/// thing in `main`, before any other thread or signal machinery exists.
#[cfg(unix)]
fn reset_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {
    // windows has no SIGPIPE; nothing to restore
}

fn main() -> ExitCode {
    reset_sigpipe();
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        print_all_subcommands();
        return ExitCode::SUCCESS;
    };
    match command {
        Command::ValidateSpec { path } => {
            let path = match resolve_dict_path(path) {
                Ok(path) => path,
                Err(err) => {
                    eprintln!("{err}");
                    return ExitCode::FAILURE;
                }
            };
            let problems = dbdict::validate_spec(&path);
            for line in problems.render() {
                eprintln!("{line}");
            }
            if problems.status().failed() {
                ExitCode::FAILURE
            } else {
                println!("{}: ok", path.display());
                ExitCode::SUCCESS
            }
        }
        Command::ValidateMeta(args) => run_validate(args, |path, table| {
            dbdict::validate_meta(path, table, &dbdict_duckdb::NativeDuckdb)
        }),
        Command::ValidateData(args) => run_validate(args, |path, table| {
            dbdict::validate_data(path, table, &dbdict_duckdb::NativeDuckdb)
        }),
        Command::Resolve { dict } => run_resolve(dict),
        Command::Ddl { dict } => run_ddl(dict),
        Command::Dummy(args) => run_dummy(args),
        Command::Spec => {
            print!("{}", dbdict::SPEC_MD);
            ExitCode::SUCCESS
        }
        Command::Types {
            command: TypesCommand::Parquet { path },
        } => match dbdict_parquet::column_type_info(&path) {
            Ok(cols) => {
                print_types_table(&cols);
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        },
        Command::Types {
            command: TypesCommand::Duckdb { path },
        } => match dbdict_duckdb::read_schema(&path) {
            Ok(schema) => {
                print_duckdb_schema(&schema);
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        },
        Command::Skill { command } => {
            let skill = match command {
                SkillCommand::Read => READ_SKILL,
                SkillCommand::Write => WRITE_SKILL,
            };
            print!("{skill}");
            ExitCode::SUCCESS
        }
    }
}

fn print_all_subcommands() {
    print!("{}", subcommands_listing());
}

/// Build the listing of all leaf subcommands, including nested ones like
/// `skill read`. The top-level `help` command is kept, but the auto-generated
/// `help` entries on each subcommand group are dropped as noise.
fn subcommands_listing() -> String {
    // `build()` injects clap's auto-generated `help` subcommand into the tree.
    let mut cmd = Cli::command();
    cmd.build();
    let mut rows = Vec::new();
    collect_subcommands(&cmd, "", &mut rows);
    let width = rows.iter().map(|(path, _)| path.len()).max().unwrap_or(0);
    let mut out = String::from("Usage: dbdict <COMMAND>\n\nCommands:\n");
    for (path, about) in rows {
        out.push_str(&format!("  {path:<width$}  {about}\n"));
    }
    out
}

fn collect_subcommands(cmd: &clap::Command, prefix: &str, rows: &mut Vec<(String, String)>) {
    for sub in cmd.get_subcommands() {
        let is_help = sub.get_name() == "help";
        // Keep only the top-level `help`; nested `help` entries are noise.
        if is_help && !prefix.is_empty() {
            continue;
        }
        let path = if prefix.is_empty() {
            sub.get_name().to_string()
        } else {
            format!("{prefix} {}", sub.get_name())
        };
        // `help` carries a mirror of the whole command tree; treat it as a leaf.
        if !is_help && sub.get_subcommands().any(|s| s.get_name() != "help") {
            collect_subcommands(sub, &path, rows);
        } else {
            let about = sub.get_about().map(|s| s.to_string()).unwrap_or_default();
            rows.push((path, about));
        }
    }
}

fn resolve_dict_path(path: Option<PathBuf>) -> Result<PathBuf, String> {
    let path = path.unwrap_or_else(|| PathBuf::from("."));
    if path.is_dir() {
        // Prefer dbdict.yaml, but fall back to the legacy data-dict.yaml name.
        let candidate = path.join("dbdict.yaml");
        if candidate.is_file() {
            Ok(candidate)
        } else {
            let legacy = path.join("data-dict.yaml");
            if legacy.is_file() {
                Ok(legacy)
            } else {
                Err(format!(
                    "no dbdict.yaml or data-dict.yaml found in {}",
                    path.display()
                ))
            }
        }
    } else {
        Ok(path)
    }
}

/// Run a meta or data validation and turn its outcome into rendered output and
/// an exit code. `validate` is a closure so the meta level can capture the
/// duckdb backend it passes into the library.
fn run_validate(
    args: ValidateArgs,
    validate: impl Fn(&Path, Option<&str>) -> ProblemSet,
) -> ExitCode {
    let dict = match resolve_dict_path(Some(args.dict)) {
        Ok(dict) => dict,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let problems = validate(&dict, args.table.as_deref());
    let status = problems.status();
    if args.json {
        println!("{}", problems_to_json(&problems));
    } else {
        for line in problems.render() {
            eprintln!("{line}");
        }
        if !status.failed() {
            println!("{}: ok", dict.display());
        }
    }
    if status.failed() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Run `resolve`: load and lower the dictionary, expand every typedef in a
/// scratch DuckDB, and print the canonical expansions. Fails when the
/// dictionary itself fails spec validation, or when any typedef won't expand
/// (unknown, cyclic, or malformed — DuckDB's own error is printed inline).
/// Shared front half of the three model-consuming subcommands (resolve, ddl,
/// dummy): resolve the optional dictionary path, load and lower it, and echo
/// any warnings a successful load still carries. On success returns the lowered
/// `DataDict`; on any failure it has already printed the reason and hands back
/// the `ExitCode` for the caller to `return`.
///
/// The `Err(ExitCode)` shape is the early-exit idiom for functions that return
/// `ExitCode` rather than `Result` — `?` isn't available, so each caller writes
/// `match load_lowered_or_exit(..) { Ok(d) => d, Err(code) => return code }` and
/// then proceeds with its own command-specific tail.
fn load_lowered_or_exit(dict: Option<PathBuf>) -> Result<DataDict, ExitCode> {
    let dict_path = match resolve_dict_path(dict) {
        Ok(dict_path) => dict_path,
        Err(err) => {
            eprintln!("{err}");
            return Err(ExitCode::FAILURE);
        }
    };
    match dbdict::load_and_lower(&dict_path) {
        Err(problems) => {
            for line in problems.render() {
                eprintln!("{line}");
            }
            Err(ExitCode::FAILURE)
        }
        Ok((problems, dict)) => {
            // an Ok load can still carry warnings — keep them visible
            for line in problems.render() {
                eprintln!("{line}");
            }
            Ok(dict)
        }
    }
}

fn run_resolve(dict: Option<PathBuf>) -> ExitCode {
    let dict = match load_lowered_or_exit(dict) {
        Ok(dict) => dict,
        Err(code) => return code,
    };
    let expansions = dbdict_duckdb::expand_typedefs(&dict);
    print_typedef_expansions(&expansions);
    if expansions.iter().any(|e| e.expansion.is_err()) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Run `ddl`: load and lower the dictionary, generate the DuckDB DDL script,
/// and print it to stdout. Problems — load errors, typedef shadowing, a
/// script that fails its scratch self-check — go to stderr with a nonzero
/// exit, like the other commands.
fn run_ddl(dict: Option<PathBuf>) -> ExitCode {
    let dict = match load_lowered_or_exit(dict) {
        Ok(dict) => dict,
        Err(code) => return code,
    };
    match dbdict_ddl::generate(&dict) {
        Ok(script) => {
            print!("{script}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

/// Parse the repeatable `--rows-table TABLE=N` entries into a map. Each entry
/// must contain a single `=`, a non-empty table name, and a row count that fits
/// in a u64. On a malformed entry, returns a message naming it (the last write
/// for a repeated table wins, matching clap's usual "last flag wins" feel).
fn parse_table_rows(entries: &[String]) -> Result<HashMap<String, u64>, String> {
    let mut map = HashMap::new();
    for entry in entries {
        let (table, count) = entry
            .split_once('=')
            .ok_or_else(|| format!("--rows-table entry `{entry}` must be TABLE=N"))?;
        // reject an empty table name here rather than let `{"": n}` flow through
        // and surface later as a confusing "table \"\" is not declared" error
        if table.is_empty() {
            return Err(format!(
                "--rows-table entry `{entry}` has an empty table name — expected TABLE=N"
            ));
        }
        // a u64 parse failure covers both non-numeric text and out-of-range
        // values (negative, too big); the message names neither specifically so
        // it never misdirects — it just says what a valid count looks like
        let count: u64 = count.parse().map_err(|_| {
            format!(
                "--rows-table entry `{entry}` has an invalid row count `{count}` — \
                 expected a non-negative whole number"
            )
        })?;
        map.insert(table.to_string(), count);
    }
    Ok(map)
}

/// Run `dummy`: load and lower the dictionary, generate dummy data, and write
/// it to a `.duckdb` file at `out`. Mirrors `run_ddl`'s error handling — load
/// errors and generation refusals go to stderr with a nonzero exit. An existing
/// `out` is refused unless `--force`; the write itself goes through a temp file
/// and a rename so a failed run never destroys an existing database.
fn run_dummy(args: DummyArgs) -> ExitCode {
    let out = args.out.as_path();
    // parse the repeatable --rows-table entries before touching the dictionary,
    // so a malformed flag fails fast regardless of the dictionary's validity
    let table_rows = match parse_table_rows(&args.table_rows) {
        Ok(table_rows) => table_rows,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let dict = match load_lowered_or_exit(args.dict) {
        Ok(dict) => dict,
        Err(code) => return code,
    };
    // fail fast before the expensive generate if the target exists and
    // we may not replace it — avoids wasting a generation pass and any
    // --sql side effect on a run that was always going to be refused
    if out.exists() && !args.force {
        eprintln!(
            "output file {} already exists — refusing to overwrite (use --force)",
            out.display()
        );
        return ExitCode::FAILURE;
    }
    let opts = dbdict_dummy_data::GenerateOptions {
        rows: args.rows,
        table_rows,
        seed: args.seed,
        null_fraction: args.null_fraction,
    };
    let generated = match dbdict_dummy_data_duckdb::generate(&dict, &opts) {
        Ok(generated) => generated,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    // optional --sql export: the exact script generate() produced — a
    // self-contained reproduction. any declared duckdb extensions lead it as
    // `LOAD` statements, so running this file on a bare duckdb rebuilds the db
    if let Some(sql_path) = args.sql.as_deref()
        && let Err(err) = std::fs::write(sql_path, &generated.script)
    {
        eprintln!("could not write {}: {err}", sql_path.display());
        return ExitCode::FAILURE;
    }
    match write_db_into_place(&generated, out) {
        Ok(()) => {
            println!("wrote dummy data to {}", out.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

/// Write the generated database into place atomically: write to a sibling temp
/// file, then rename it over `out`. Renaming avoids the window a naive
/// delete-then-write would open — if the write fails, the existing `out` is left
/// untouched (the library's own `write_db` refuses to touch an existing path for
/// the same reason; this is how the CLI honors `--force` without that risk).
/// Rename-over-existing is atomic on unix, the target platform.
fn write_db_into_place(
    generated: &dbdict_dummy_data_duckdb::Generated,
    out: &Path,
) -> Result<(), String> {
    // a sibling temp on the same filesystem, so the rename below is a cheap
    // metadata move rather than a copy across devices; the pid keeps concurrent
    // runs writing to the same --out from colliding on the temp name
    let tmp = out.with_extension(format!("tmp-{}.duckdb", std::process::id()));
    // clear leftovers from a previously killed run of this pid before writing
    // (write_db refuses a path that already exists)
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(wal_path(&tmp));
    if let Err(err) = generated.write_db(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.to_string());
    }
    // Inferred: duckdb keeps a write-ahead log beside a database as `<file>.wal`.
    // a best-effort remove of any stale sidecar at the target keeps it from being
    // replayed into the database we are about to rename into place; harmless if
    // absent or (were the convention ever different) misnamed — it just no-ops
    let _ = std::fs::remove_file(wal_path(out));
    std::fs::rename(&tmp, out).map_err(|err| {
        let _ = std::fs::remove_file(&tmp);
        format!("could not move the generated database into place: {err}")
    })
}

/// The path duckdb uses for a database's write-ahead log: the database path with
/// `.wal` appended (see [`write_db_into_place`] for the sourcing caveat).
fn wal_path(db: &Path) -> PathBuf {
    let mut wal = db.as_os_str().to_owned();
    wal.push(".wal");
    PathBuf::from(wal)
}

/// Print typedef expansions as `name  declared-expression  → canonical`: the
/// globals first, then each table's entries under a `table <name>:` heading.
/// `expand_typedefs` returns them already in that order.
fn print_typedef_expansions(expansions: &[dbdict_duckdb::TypedefExpansion]) {
    if expansions.is_empty() {
        println!("(no typedefs)");
        return;
    }
    // one width across all groups: simpler than a per-group pass, at the
    // cost of some padding when one group's names run long
    let name_width = expansions.iter().map(|e| e.name.len()).max().unwrap_or(0);
    let expr_width = expansions.iter().map(|e| e.expr.len()).max().unwrap_or(0);
    let mut printed_any = false;
    let mut current_table: Option<&str> = None;
    for e in expansions {
        // a let-chain: the heading prints once per table, when its first
        // entry arrives (globals have no table and no heading)
        if let Some(table) = e.table.as_deref()
            && current_table != Some(table)
        {
            current_table = Some(table);
            if printed_any {
                println!(); // separator, only when a group came before
            }
            println!("table {table}:");
        }
        let indent = if e.table.is_some() { "  " } else { "" };
        match &e.expansion {
            Ok(canonical) => println!(
                "{indent}{:<name_width$}  {:<expr_width$}  → {canonical}",
                e.name, e.expr
            ),
            Err(error) => println!(
                "{indent}{:<name_width$}  {:<expr_width$}  → error: {error}",
                e.name, e.expr
            ),
        }
        printed_any = true;
    }
}

/// Print a DuckDB database's schema: every table or view with its columns'
/// canonical types.
fn print_duckdb_schema(schema: &[dbdict::rich::TableSchema]) {
    if schema.is_empty() {
        println!("(no tables or views)");
        return;
    }
    let mut first = true;
    for table in schema {
        if !first {
            println!();
        }
        first = false;
        println!("{}", table.name);
        let headers = ["#", "column", "type"];
        let num_width = table.columns.len().to_string().len().max(headers[0].len());
        let name_width = table
            .columns
            .iter()
            .map(|(name, _)| name.len())
            .max()
            .unwrap_or(0)
            .max(headers[1].len());
        let type_width = table
            .columns
            .iter()
            .map(|(_, column_type)| column_type.len())
            .max()
            .unwrap_or(0)
            .max(headers[2].len());
        println!(
            "  {:<num_width$}  {:<name_width$}  {}",
            headers[0], headers[1], headers[2]
        );
        // a rule under the header row, matching the parquet printer's style
        println!("  {}", "─".repeat(num_width + name_width + type_width + 4));
        for (i, (name, column_type)) in table.columns.iter().enumerate() {
            println!(
                "  {:<num_width$}  {:<name_width$}  {column_type}",
                i + 1,
                name
            );
        }
    }
}

fn problems_to_json(problems: &ProblemSet) -> serde_json::Value {
    let items: Vec<serde_json::Value> = problems
        .items
        .iter()
        .map(|p| {
            let mut value = serde_json::to_value(p).expect("a Problem always serializes");
            if let Some(location) = p.location(&problems.source) {
                value["location"] = serde_json::to_value(location).expect("location serializes");
            }
            value
        })
        .collect();
    serde_json::json!({
        "status": problems.status(),
        "problems": items,
    })
}

fn print_types_table(cols: &[dbdict_parquet::ColumnTypeInfo]) {
    let headers = ["#", "column", "dict type", "logical type", "physical type"];
    let num_width = cols.len().to_string().len().max(headers[0].len());
    let widths = [
        num_width,
        cols.iter()
            .map(|c| c.name.len())
            .max()
            .unwrap_or(0)
            .max(headers[1].len()),
        cols.iter()
            .map(|c| c.dict_type.len())
            .max()
            .unwrap_or(0)
            .max(headers[2].len()),
        cols.iter()
            .map(|c| c.logical_type.as_deref().unwrap_or("").len())
            .max()
            .unwrap_or(0)
            .max(headers[3].len()),
        cols.iter()
            .map(|c| c.physical_type.len())
            .max()
            .unwrap_or(0)
            .max(headers[4].len()),
    ];

    println!(
        "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}",
        headers[0],
        headers[1],
        headers[2],
        headers[3],
        headers[4],
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
        w3 = widths[3],
        w4 = widths[4],
    );
    println!("{}", "─".repeat(widths.iter().sum::<usize>() + 8));

    for (i, col) in cols.iter().enumerate() {
        println!(
            "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}",
            i + 1,
            col.name,
            col.dict_type,
            col.logical_type.as_deref().unwrap_or(""),
            col.physical_type,
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
            w3 = widths[3],
            w4 = widths[4],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dbdict-cli-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_table_rows_parses_valid_entries() {
        let entries = vec!["trades=100".to_string(), "categories=3".to_string()];
        let map = parse_table_rows(&entries).unwrap();
        assert_eq!(map.get("trades"), Some(&100));
        assert_eq!(map.get("categories"), Some(&3));
    }

    #[test]
    fn parse_table_rows_rejects_empty_table_name() {
        // `=5` has no table before the `=`; that is a malformed flag, not a
        // request to override a table literally named "" (which would only
        // surface later as a confusing UnknownTableOverride)
        let err = parse_table_rows(&["=5".to_string()]).unwrap_err();
        assert!(err.contains("=5"), "error should name the bad entry: {err}");
        assert!(
            !err.contains("non-numeric"),
            "an empty table name is not a numeric problem: {err}"
        );
    }

    #[test]
    fn parse_table_rows_reports_out_of_range_count_accurately() {
        // -5 and a huge value are numeric but not valid u64 row counts; the
        // message must not claim they are "non-numeric" (misdirects the user)
        let neg = parse_table_rows(&["trades=-5".to_string()]).unwrap_err();
        assert!(neg.contains("trades=-5"), "should name the entry: {neg}");
        assert!(
            !neg.contains("non-numeric"),
            "-5 is numeric but out of range, not non-numeric: {neg}"
        );
    }

    #[test]
    fn explicit_file_is_returned_as_is() {
        let dir = temp_dir("file");
        let file = dir.join("custom.yaml");
        fs::write(&file, "tables: []\n").unwrap();
        assert_eq!(resolve_dict_path(Some(file.clone())).unwrap(), file);
    }

    #[test]
    fn directory_resolves_to_dbdict_yaml() {
        let dir = temp_dir("dir");
        let dict = dir.join("dbdict.yaml");
        fs::write(&dict, "tables: []\n").unwrap();
        assert_eq!(resolve_dict_path(Some(dir)).unwrap(), dict);
    }

    #[test]
    fn directory_resolves_to_legacy_data_dict_yaml() {
        // The legacy data-dict.yaml name is still resolved as a fallback.
        let dir = temp_dir("legacy");
        let dict = dir.join("data-dict.yaml");
        fs::write(&dict, "tables: []\n").unwrap();
        assert_eq!(resolve_dict_path(Some(dir)).unwrap(), dict);
    }

    #[test]
    fn dbdict_yaml_wins_over_legacy_when_both_present() {
        let dir = temp_dir("both");
        let dbdict = dir.join("dbdict.yaml");
        let legacy = dir.join("data-dict.yaml");
        fs::write(&dbdict, "tables: []\n").unwrap();
        fs::write(&legacy, "tables: []\n").unwrap();
        assert_eq!(resolve_dict_path(Some(dir)).unwrap(), dbdict);
    }

    #[test]
    fn directory_without_dbdict_yaml_errors() {
        let dir = temp_dir("empty");
        let err = resolve_dict_path(Some(dir.clone())).unwrap_err();
        assert!(err.contains("no dbdict.yaml or data-dict.yaml found"));
        assert!(err.contains(&dir.display().to_string()));
    }

    #[test]
    fn none_defaults_to_current_directory() {
        assert_eq!(resolve_dict_path(None), resolve_dict_path(Some(".".into())));
    }

    #[test]
    fn nonexistent_file_is_returned_as_is() {
        // A path that is neither a dir nor an existing file is passed through
        // so the caller surfaces the real read error.
        let path = PathBuf::from("does-not-exist.yaml");
        assert_eq!(resolve_dict_path(Some(path.clone())).unwrap(), path);
    }

    /// Validate a dictionary that is clean apart from a S09 ($learn_more)
    /// warning, returning its problems.
    fn warning_problems(name: &str) -> ProblemSet {
        let dir = temp_dir(name);
        let dict = dir.join("dbdict.yaml");
        fs::write(&dict, "$version: 0.1.0\n").unwrap();
        dbdict::validate_spec(&dict)
    }

    #[test]
    fn json_carries_problems_on_success() {
        // A warning-only set still passes, but its status reflects the warning.
        let json = problems_to_json(&warning_problems("json-ok"));
        assert_eq!(json["status"], "warning");
        assert_eq!(json["problems"][0]["code"], "S09");
        assert_eq!(json["problems"][0]["severity"], "warning");
        assert_eq!(json["problems"][0]["kind"], "spec");
        assert!(
            json["problems"][0]["expected"]
                .as_str()
                .is_some_and(|e| e.contains("$learn_more")),
            "S09 expectation should be carried in the JSON output"
        );
        // The span resolves to a 0-based (LSP) line/column range so an editor
        // can place the diagnostic in the file.
        let location = &json["problems"][0]["location"];
        assert_eq!(location["start_line"], 0);
        assert_eq!(location["start_column"], 0);
    }

    #[test]
    fn json_reports_error_status() {
        let problems = ProblemSet::from_preflight(
            dbdict::ProblemKind::TableNotFound {
                available: vec!["a".to_string(), "b".to_string()],
            },
            "table \"x\" is not in the data dictionary",
        );
        let json = problems_to_json(&problems);
        assert_eq!(json["status"], "error");
        assert_eq!(json["problems"][0]["kind"], "table_not_found");
        assert_eq!(json["problems"][0]["available"][1], "b");
    }
}
