use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use dbdict::ProblemSet;

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
        Command::ValidateData(args) => run_validate(args, dbdict::validate_data),
        Command::Resolve { dict } => run_resolve(dict),
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
fn run_resolve(dict: Option<PathBuf>) -> ExitCode {
    let dict_path = match resolve_dict_path(dict) {
        Ok(dict_path) => dict_path,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    match dbdict::load_and_lower(&dict_path) {
        Err(problems) => {
            for line in problems.render() {
                eprintln!("{line}");
            }
            ExitCode::FAILURE
        }
        Ok((problems, dict)) => {
            // an Ok load can still carry warnings — keep them visible
            for line in problems.render() {
                eprintln!("{line}");
            }
            let expansions = dbdict_duckdb::expand_typedefs(&dict);
            print_typedef_expansions(&expansions);
            if expansions.iter().any(|e| e.expansion.is_err()) {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
    }
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
