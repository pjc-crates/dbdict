//! Integration tests that run the `dbdict` binary end to end.

use std::path::PathBuf;
use std::process::Command;

/// Running `dbdict` with no arguments lists every subcommand, including
/// nested ones like `skill read`.
///
/// When this snapshot changes (i.e. the set of commands changes), update the
/// command listing under "## The CLI" in the repo-root README.md to match.
#[test]
fn no_args_lists_all_subcommands() {
    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .output()
        .expect("failed to run dbdict");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is not valid UTF-8");
    insta::assert_snapshot!(stdout);
}

/// A fixture that fails schema validation with two errors (S07, S08) and a warning (S09),
/// in that emission order. Validating its data skips the data comparison (the
/// dictionary has errors), so no source is ever read.
fn multi_error_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multi-error-with-warning.yaml")
}

/// The default (text) output renders every diagnostic — both errors and the
/// warning — to stderr, in emission order.
#[test]
fn multiple_diagnostics_text_output() {
    let fixture = multi_error_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .args(["validate-data"])
        .arg(&fixture)
        .output()
        .expect("failed to run dbdict");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr is not valid UTF-8");
    insta::assert_snapshot!(sanitize(&stderr, &fixture.display().to_string()));
}

/// The `--json` output carries the same diagnostics as a structured array,
/// preserving severity, code, and emission order.
#[test]
fn multiple_diagnostics_json_output() {
    let fixture = multi_error_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .args(["validate-data"])
        .arg(&fixture)
        .arg("--json")
        .output()
        .expect("failed to run dbdict");
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is not valid UTF-8");
    // Re-serialize so the snapshot is pretty-printed and key order is stable.
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("stdout is valid JSON");
    insta::assert_snapshot!(serde_json::to_string_pretty(&value).unwrap());
}

/// The whole rich round-trip through the binary: a rich dictionary validated
/// against a real duckdb database file, cleanly matching.
#[test]
fn validate_meta_rich_round_trip_succeeds() {
    let dir = std::env::temp_dir().join(format!("dbdict-cli-rich-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let conn = duckdb::Connection::open(dir.join("warehouse.duckdb")).expect("create db");
    conn.execute_batch("CREATE TABLE trades (qty BIGINT, price DECIMAL(12,2));")
        .expect("create schema");
    drop(conn); // flush and close before the binary opens it read-only

    let dict = dir.join("dbdict.yaml");
    std::fs::write(
        &dict,
        indoc::indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            typedef:
              money: DECIMAL(12, 2)
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                columns:
                  - name: qty
                    type: BIGINT
                  - name: price
                    type: money
        "#},
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("validate-meta")
        .arg(&dict)
        .output()
        .expect("failed to run dbdict");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected a clean round-trip, stderr:\n{stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ok"), "got {stdout:?}");
}

/// A fresh temp dir for a test that builds its own fixture files.
fn temp_fixture_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dbdict-cli-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `resolve` prints globals, then per-table groups, as
/// `name  declared  → canonical` — the format users script against.
#[test]
fn resolve_prints_global_and_scoped_expansions() {
    let dir = temp_fixture_dir("resolve-ok");
    let dict = dir.join("dbdict.yaml");
    std::fs::write(
        &dict,
        indoc::indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            typedef:
              money: DECIMAL(12, 2)
              address: STRUCT(city VARCHAR, postcode INTEGER)
            tables:
              - name: trades
                typedef:
                  money: DECIMAL(18, 4)
                columns:
                  - name: price
                    type: money
        "#},
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("resolve")
        .arg(&dict)
        .output()
        .expect("failed to run dbdict");
    assert!(output.status.success(), "expected success: {output:?}");
    let stdout = String::from_utf8(output.stdout).expect("stdout is not valid UTF-8");
    insta::assert_snapshot!(stdout);
}

/// A typedef duckdb rejects fails `resolve` and names the problem inline.
#[test]
fn resolve_fails_on_a_broken_typedef() {
    let dir = temp_fixture_dir("resolve-broken");
    let dict = dir.join("dbdict.yaml");
    std::fs::write(
        &dict,
        indoc::indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            typedef:
              dangling: NO_SUCH_TYPE
            tables: []
        "#},
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("resolve")
        .arg(&dict)
        .output()
        .expect("failed to run dbdict");
    assert!(!output.status.success(), "a broken typedef must fail");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // duckdb's message is not snapshotted (it may change across versions);
    // the contract is: the entry is printed with an inline error
    assert!(stdout.contains("dangling"), "got {stdout:?}");
    assert!(stdout.contains("error:"), "got {stdout:?}");
}

/// A legacy (0.1.0) dictionary has no `typedef:` key to resolve; the command
/// succeeds and says so rather than failing.
#[test]
fn resolve_on_a_legacy_dictionary_reports_no_typedefs() {
    let dir = temp_fixture_dir("resolve-legacy");
    let dict = dir.join("data-dict.yaml");
    std::fs::write(
        &dict,
        indoc::indoc! {r#"
            $version: "0.1.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            tables: []
        "#},
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("resolve")
        .arg(&dict)
        .output()
        .expect("failed to run dbdict");
    assert!(output.status.success(), "expected success: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("(no typedefs)"), "got {stdout:?}");
}

/// `types duckdb` lists every relation with its canonical column types.
#[test]
fn types_duckdb_prints_every_relation() {
    let dir = temp_fixture_dir("types-duckdb");
    let db = dir.join("warehouse.duckdb");
    let conn = duckdb::Connection::open(&db).expect("create db");
    conn.execute_batch(
        "CREATE TABLE trades (qty BIGINT, price DECIMAL(12,2));
         CREATE TABLE orders (id INTEGER, home STRUCT(city VARCHAR, postcode INTEGER));",
    )
    .expect("create schema");
    drop(conn); // flush and close before the binary opens it read-only

    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("types")
        .arg("duckdb")
        .arg(&db)
        .output()
        .expect("failed to run dbdict");
    assert!(output.status.success(), "expected success: {output:?}");
    let stdout = String::from_utf8(output.stdout).expect("stdout is not valid UTF-8");
    insta::assert_snapshot!(stdout);
}

/// A small rich fixture exercising D01 (required), D02 (primary_key), D04
/// (foreign_key) plus a many-to-one relationship. `source.file` points at
/// `gen.duckdb` so that, once `dummy -o <dir>/gen.duckdb` has written it,
/// `validate-data <dict>` reads back exactly what was generated. Returns the
/// dict path; the caller owns the directory.
fn dummy_fixture(dir: &std::path::Path) -> PathBuf {
    let dict = dir.join("dbdict.yaml");
    std::fs::write(
        &dict,
        indoc::indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            source:
              duckdb:
                file: gen.duckdb
            tables:
              - name: categories
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
              - name: trades
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: qty
                    type: BIGINT
                    constraints: [required]
                  - name: cat_id
                    type: BIGINT
                    constraints: [foreign_key]
            relationships:
              - join: trades.cat_id = categories.id
                cardinality: many-to-one
        "#},
    )
    .unwrap();
    dict
}

/// The generator's headline promise, proven end to end through the binary:
/// `dummy` writes a `.duckdb` file, and `validate-data` on the same
/// dictionary — the built-in oracle — passes on it. If the generated data
/// satisfied the constraints only in the library's own tests but the CLI
/// wired something up wrong, this catches it.
#[test]
fn dummy_generates_a_database_that_validates() {
    let dir = temp_fixture_dir("dummy-ok");
    let dict = dummy_fixture(&dir);
    let out = dir.join("gen.duckdb");

    let gen_out = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("dummy")
        .arg(&dict)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("failed to run dbdict");
    let stderr = String::from_utf8_lossy(&gen_out.stderr);
    assert!(gen_out.status.success(), "dummy failed, stderr:\n{stderr}");
    assert!(out.is_file(), "the output database was not written");

    // the oracle: validate the generated database through the binary
    let check = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("validate-data")
        .arg(&dict)
        .output()
        .expect("failed to run dbdict");
    let check_stderr = String::from_utf8_lossy(&check.stderr);
    assert!(
        check.status.success(),
        "generated data failed validate-data, stderr:\n{check_stderr}"
    );

    // validate-data passes vacuously on empty tables, so also confirm rows were
    // actually generated at the DEFAULT row count (this is the only test that
    // exercises it) — a regression to 0 rows would otherwise slip through green
    let conn = duckdb::Connection::open(&out).expect("open generated db");
    let trades: i64 = conn
        .query_row("SELECT count(*) FROM trades", [], |r| r.get(0))
        .expect("count trades");
    let categories: i64 = conn
        .query_row("SELECT count(*) FROM categories", [], |r| r.get(0))
        .expect("count categories");
    assert_eq!(trades, 10, "default --rows is 10");
    assert_eq!(categories, 10, "default --rows is 10");
}

/// `--sql <file>` also writes the generated script (DDL + INSERTs) — the same
/// text `write_db` executes — so a user can inspect exactly what built the
/// database.
#[test]
fn dummy_sql_export_writes_the_script() {
    let dir = temp_fixture_dir("dummy-sql");
    let dict = dummy_fixture(&dir);
    let out = dir.join("gen.duckdb");
    let sql = dir.join("gen.sql");

    let gen_out = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("dummy")
        .arg(&dict)
        .arg("-o")
        .arg(&out)
        .arg("--sql")
        .arg(&sql)
        .output()
        .expect("failed to run dbdict");
    let stderr = String::from_utf8_lossy(&gen_out.stderr);
    assert!(gen_out.status.success(), "dummy failed, stderr:\n{stderr}");

    let script = std::fs::read_to_string(&sql).expect("sql export was not written");
    assert!(
        script.contains("CREATE TABLE"),
        "no DDL in export:\n{script}"
    );
    assert!(
        script.contains("INSERT INTO"),
        "no INSERTs in export:\n{script}"
    );
    // must be THIS dictionary's script, not a generic/stale template — check it
    // names the fixture's own tables
    assert!(
        script.contains("categories") && script.contains("trades"),
        "export should reference the fixture's tables:\n{script}"
    );
}

/// `--force` overwrites an existing `--out`. Generate once with 3 rows, then
/// again with `--force` and 7 rows: the second run succeeds, still validates,
/// and — crucially — the on-disk database reflects the SECOND run (7 rows),
/// proving the file was actually rewritten rather than left stale.
#[test]
fn dummy_force_overwrites_existing_output() {
    let dir = temp_fixture_dir("dummy-force");
    let dict = dummy_fixture(&dir);
    let out = dir.join("gen.duckdb");

    let first = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("dummy")
        .arg(&dict)
        .arg("-o")
        .arg(&out)
        .arg("--rows")
        .arg("3")
        .output()
        .expect("failed to run dbdict");
    assert!(first.status.success(), "first generate should succeed");

    let second = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("dummy")
        .arg(&dict)
        .arg("-o")
        .arg(&out)
        .arg("--force")
        .arg("--rows")
        .arg("7")
        .output()
        .expect("failed to run dbdict");
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        second.status.success(),
        "--force should overwrite:\n{stderr}"
    );

    let check = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("validate-data")
        .arg(&dict)
        .output()
        .expect("failed to run dbdict");
    assert!(
        check.status.success(),
        "overwritten db should still validate"
    );

    // the second run's 7 rows must be what's on disk — not the first run's 3
    let conn = duckdb::Connection::open(&out).expect("open overwritten db");
    let trades: i64 = conn
        .query_row("SELECT count(*) FROM trades", [], |r| r.get(0))
        .expect("count trades");
    assert_eq!(
        trades, 7,
        "database must reflect the --force rewrite, not the stale first run"
    );
}

/// Without `--force`, `dummy` refuses to clobber an existing `--out`: the
/// second run fails and says the file already exists. Locks the guard that
/// keeps a real database from being silently overwritten.
#[test]
fn dummy_refuses_existing_output() {
    let dir = temp_fixture_dir("dummy-existing");
    let dict = dummy_fixture(&dir);
    let out = dir.join("gen.duckdb");

    let first = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("dummy")
        .arg(&dict)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("failed to run dbdict");
    assert!(first.status.success(), "first generate should succeed");

    let second = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("dummy")
        .arg(&dict)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("failed to run dbdict");
    assert!(!second.status.success(), "a second run must refuse");
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(stderr.contains("already exists"), "got:\n{stderr}");
}

/// `--rows` sets the global row count and `--rows-table TABLE=N` overrides one
/// table. Generate with both, then query the database back: the overridden
/// table has its own count, the other falls back to `--rows`.
#[test]
fn dummy_row_count_flags_control_table_sizes() {
    let dir = temp_fixture_dir("dummy-rows");
    let dict = dummy_fixture(&dir);
    let out = dir.join("gen.duckdb");

    let gen_out = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("dummy")
        .arg(&dict)
        .arg("-o")
        .arg(&out)
        .arg("--rows")
        .arg("4")
        .arg("--rows-table")
        .arg("categories=3")
        .output()
        .expect("failed to run dbdict");
    let stderr = String::from_utf8_lossy(&gen_out.stderr);
    assert!(gen_out.status.success(), "dummy failed, stderr:\n{stderr}");

    let conn = duckdb::Connection::open(&out).expect("open generated db");
    let cats: i64 = conn
        .query_row("SELECT count(*) FROM categories", [], |r| r.get(0))
        .expect("count categories");
    let trades: i64 = conn
        .query_row("SELECT count(*) FROM trades", [], |r| r.get(0))
        .expect("count trades");
    assert_eq!(cats, 3, "categories should use the --rows-table override");
    assert_eq!(trades, 4, "trades should fall back to --rows");
}

/// `--seed` is passed through to the generator: two runs with different seeds
/// produce different scripts, and the same seed reproduces byte-for-byte.
/// Compared through the `--sql` export, which is the deterministic artifact.
#[test]
fn dummy_seed_is_passed_through() {
    let dir = temp_fixture_dir("dummy-seed");
    let dict = dummy_fixture(&dir);
    let out = dir.join("gen.duckdb");

    let export = |seed: &str, name: &str| -> String {
        let sql = dir.join(name);
        let run = Command::new(env!("CARGO_BIN_EXE_dbdict"))
            .arg("dummy")
            .arg(&dict)
            .arg("-o")
            .arg(&out)
            .arg("--force")
            .arg("--sql")
            .arg(&sql)
            .arg("--seed")
            .arg(seed)
            .output()
            .expect("failed to run dbdict");
        assert!(run.status.success(), "dummy --seed {seed} failed");
        std::fs::read_to_string(&sql).expect("sql export written")
    };

    let seed1_a = export("1", "s1a.sql");
    let seed1_b = export("1", "s1b.sql");
    let seed2 = export("2", "s2.sql");
    assert_eq!(seed1_a, seed1_b, "same seed must reproduce the same script");
    assert_ne!(seed1_a, seed2, "different seeds must differ");
}

/// `--null-fraction` is wired through to the generator: 0.0 is accepted and
/// generates cleanly, and an out-of-range value is refused by the library and
/// surfaced by the CLI. Proves the flag reaches GenerateOptions.null_fraction
/// rather than being silently ignored.
#[test]
fn dummy_null_fraction_flag_is_wired() {
    let dir = temp_fixture_dir("dummy-nullfrac");
    let dict = dummy_fixture(&dir);

    let ok = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("dummy")
        .arg(&dict)
        .arg("-o")
        .arg(dir.join("zero.duckdb"))
        .arg("--null-fraction")
        .arg("0")
        .output()
        .expect("failed to run dbdict");
    let stderr = String::from_utf8_lossy(&ok.stderr);
    assert!(
        ok.status.success(),
        "--null-fraction 0 should work:\n{stderr}"
    );

    let bad = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("dummy")
        .arg(&dict)
        .arg("-o")
        .arg(dir.join("bad.duckdb"))
        .arg("--null-fraction")
        .arg("1.5")
        .output()
        .expect("failed to run dbdict");
    assert!(!bad.status.success(), "an out-of-range fraction must fail");
    let stderr = String::from_utf8_lossy(&bad.stderr);
    assert!(stderr.contains("null fraction"), "got:\n{stderr}");
}

/// A legacy (0.1.0) dictionary has no DuckDB types to generate from; `dummy`
/// refuses with a clear message and writes no output file. The fixture is
/// otherwise spec-valid (the `number` column carries `examples`) so the
/// refusal comes from the generator, not from a spec-level error first.
#[test]
fn dummy_refuses_a_legacy_dictionary() {
    let dir = temp_fixture_dir("dummy-legacy");
    let dict = dir.join("data-dict.yaml");
    std::fs::write(
        &dict,
        indoc::indoc! {r#"
            $version: 0.1.0
            $learn_more: https://github.com/pjc-wspace/dbdict
            tables:
              - name: trades
                columns:
                  - name: qty
                    type: number
                    examples: [1, 2, 3]
        "#},
    )
    .unwrap();
    let out = dir.join("gen.duckdb");

    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("dummy")
        .arg(&dict)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("failed to run dbdict");
    assert!(!output.status.success(), "legacy must refuse");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("rich"), "got:\n{stderr}");
    assert!(!out.exists(), "no database should be written on refusal");
}

/// Strip terminal styling (ANSI SGR escapes and OSC-8 hyperlinks) and rewrite
/// the fixture's absolute path to a stable placeholder, so the rendered
/// diagnostic can be snapshotted.
fn sanitize(s: &str, fixture_path: &str) -> String {
    strip_terminal_escapes(s).replace(fixture_path, "<fixture>")
}

/// Remove ANSI SGR sequences (`ESC [ ... m`) and OSC-8 hyperlink wrappers
/// (`ESC ] 8 ; ; ... BEL|ST`) while leaving the visible text intact.
fn strip_terminal_escapes(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'[' => {
                    // CSI: run until a final byte in 0x40..=0x7e.
                    i += 2;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    i += 1; // consume the final byte
                }
                b']' => {
                    // OSC: run until BEL or ST (ESC \).
                    i += 2;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => i += 2,
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("stripping ASCII escapes preserves UTF-8")
}

/// Closing the read end of a pipe mid-output (`dbdict spec | head`) must not
/// panic. Rust's runtime ignores SIGPIPE before `main`, so `println!` sees
/// EPIPE and panics with "failed printing to stdout"; the binary restores the
/// default SIGPIPE disposition so it dies quietly like other unix CLIs.
///
/// The read end is closed immediately after spawn — before the child gets
/// through startup — so its very first write hits the closed pipe.
#[cfg(unix)]
#[test]
fn spec_dies_quietly_when_stdout_pipe_closes() {
    use std::os::unix::process::ExitStatusExt;
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("spec")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn dbdict");
    drop(child.stdout.take()); // close the read end, like `head -1` exiting
    let output = child.wait_with_output().expect("failed to wait for dbdict");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panic"),
        "dbdict panicked on a closed pipe:\n{stderr}"
    );
    // dying of SIGPIPE (signal 13) is the classic quiet death; finishing all
    // writes before the close (clean exit) would also be fine
    assert!(
        output.status.signal() == Some(13) || output.status.success(),
        "unexpected exit status: {:?}",
        output.status
    );
}

/// The rich data level through the binary: seeded D01 (nulls in a required
/// column), D02 (duplicated primary key), D03 (duplicated value in a `unique`
/// column), D04 (orphaned `foreign_key` value), and D05 (cardinality
/// violation on a range join, where one trade date falls inside two
/// overlapping periods) violations all report, with their codes, and fail
/// the run. The other two trade dates match exactly one and zero periods —
/// neither violates, so the D05 count stays at 1.
#[test]
fn validate_data_rich_reports_d01_through_d05() {
    let dir = temp_fixture_dir("rich-data");

    let conn = duckdb::Connection::open(dir.join("warehouse.duckdb")).expect("create db");
    conn.execute_batch(
        "CREATE TABLE trades (id BIGINT, qty BIGINT, ref VARCHAR, cat_id BIGINT, ts DATE);
         INSERT INTO trades VALUES
           (1, 10, 'ord-1', 1, DATE '2024-01-05'),
           (1, NULL, 'ord-1', 1, DATE '2024-02-15'),
           (2, 20, 'ord-2', 99, DATE '2024-01-20');
         CREATE TABLE categories (id BIGINT);
         INSERT INTO categories VALUES (1);
         CREATE TABLE periods (start DATE, \"end\" DATE);
         INSERT INTO periods VALUES
           (DATE '2024-01-01', DATE '2024-01-31'),
           (DATE '2024-01-04', DATE '2024-01-10');",
    )
    .expect("create fixture");
    drop(conn); // flush and close before the binary opens it read-only

    let dict = dir.join("dbdict.yaml");
    std::fs::write(
        &dict,
        indoc::indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: qty
                    type: BIGINT
                    constraints: [required]
                  - name: ref
                    type: VARCHAR
                    constraints: [unique]
                  - name: cat_id
                    type: BIGINT
                    constraints: [foreign_key]
                  - name: ts
                    type: DATE
              - name: categories
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
              - name: periods
                columns:
                  - name: start
                    type: DATE
                    constraints: [unique]
                  - name: end
                    type: DATE
            relationships:
              - join: trades.cat_id = categories.id
                cardinality: many-to-one
              - join: trades.ts >= periods.start AND trades.ts <= periods.end
                cardinality: many-to-one
        "#},
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("validate-data")
        .arg(&dict)
        .output()
        .expect("failed to run dbdict");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    insta::assert_snapshot!(sanitize(&stderr, &dict.display().to_string()));
}

/// The same fixture without the violations passes the data level cleanly.
/// The `unique` column holds distinct values plus *repeated NULLs* — locking
/// end to end that D03 follows SQL UNIQUE semantics (NULLs compare as
/// distinct, so an optional-but-unique column may hold many). The
/// `foreign_key` column holds present values plus a NULL — locking end to end
/// that D04 excludes NULLs (MATCH SIMPLE: NULL means "no reference"). The
/// range join's periods don't overlap, and the row with a NULL `ts` matches
/// nothing — locking end to end that D05 passes NULL join columns (SQL
/// comparison semantics) and never treats zero matches as a violation.
#[test]
fn validate_data_rich_clean_passes() {
    let dir = temp_fixture_dir("rich-data-clean");

    let conn = duckdb::Connection::open(dir.join("warehouse.duckdb")).expect("create db");
    conn.execute_batch(
        "CREATE TABLE trades (id BIGINT, qty BIGINT, ref VARCHAR, cat_id BIGINT, ts DATE);
         INSERT INTO trades VALUES
           (1, 10, 'ord-1', 1, DATE '2024-01-05'),
           (2, 20, NULL, 2, DATE '2024-01-15'),
           (3, 30, NULL, NULL, NULL);
         CREATE TABLE categories (id BIGINT);
         INSERT INTO categories VALUES (1), (2);
         CREATE TABLE periods (start DATE, \"end\" DATE);
         INSERT INTO periods VALUES
           (DATE '2024-01-01', DATE '2024-01-10'),
           (DATE '2024-01-11', DATE '2024-01-20');",
    )
    .expect("create fixture");
    drop(conn);

    let dict = dir.join("dbdict.yaml");
    std::fs::write(
        &dict,
        indoc::indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
                  - name: qty
                    type: BIGINT
                    constraints: [required]
                  - name: ref
                    type: VARCHAR
                    constraints: [unique]
                  - name: cat_id
                    type: BIGINT
                    constraints: [foreign_key]
                  - name: ts
                    type: DATE
              - name: categories
                columns:
                  - name: id
                    type: BIGINT
                    constraints: [primary_key]
              - name: periods
                columns:
                  - name: start
                    type: DATE
                    constraints: [unique]
                  - name: end
                    type: DATE
            relationships:
              - join: trades.cat_id = categories.id
                cardinality: many-to-one
              - join: trades.ts >= periods.start AND trades.ts <= periods.end
                cardinality: many-to-one
        "#},
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("validate-data")
        .arg(&dict)
        .output()
        .expect("failed to run dbdict");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stderr:\n{stderr}");
}

/// `ddl` prints an executable DuckDB script — CREATE TYPE per typedef in
/// dependency order, then CREATE TABLE per table — the format users pipe
/// into `duckdb`.
#[test]
fn ddl_prints_an_executable_script() {
    let dir = temp_fixture_dir("ddl-ok");
    let dict = dir.join("dbdict.yaml");
    std::fs::write(
        &dict,
        indoc::indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            typedef:
              big: money[]
              money: DECIMAL(18, 4)
            tables:
              - name: trades
                columns:
                  - name: qty
                    type: BIGINT
                  - name: prices
                    type: big
              - name: memos
                columns:
                  - name: body
        "#},
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("ddl")
        .arg(&dict)
        .output()
        .expect("failed to run dbdict");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stderr:\n{stderr}");
    let stdout = String::from_utf8(output.stdout).expect("stdout is not valid UTF-8");
    insta::assert_snapshot!(stdout);
}

/// Table-scoped typedef shadowing cannot be spelled in one flat script; `ddl`
/// refuses with a clear error instead of generating something broken.
#[test]
fn ddl_refuses_shadowed_typedefs() {
    let dir = temp_fixture_dir("ddl-shadowed");
    let dict = dir.join("dbdict.yaml");
    std::fs::write(
        &dict,
        indoc::indoc! {r#"
            $version: "0.2.0"
            $learn_more: https://github.com/pjc-wspace/dbdict
            typedef:
              money: DECIMAL(18, 4)
            tables:
              - name: trades
                typedef:
                  money: DECIMAL(12, 2)
                columns:
                  - name: price
                    type: money
        "#},
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("ddl")
        .arg(&dict)
        .output()
        .expect("failed to run dbdict");
    assert!(!output.status.success(), "shadowing must refuse");
    // nothing usable goes to stdout; the reason goes to stderr, naming the
    // typedef and both definition sites
    assert!(output.stdout.is_empty(), "got {:?}", output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("money"), "got: {stderr}");
    assert!(stderr.contains("trades"), "got: {stderr}");
}

/// A legacy (0.1.0) dictionary has no DuckDB types to generate from; `ddl`
/// says so and fails rather than emitting broken SQL. The fixture is otherwise
/// spec-valid (the `number` column carries `examples`, which S07 requires) so
/// the refusal comes from the *generator's* legacy check, not from a spec-level
/// error at an earlier layer — and the assertion targets a distinctive phrase
/// from the refusal message, not the bare word "legacy" (which also appears in
/// the fixture's temp-dir path and would pass on the wrong output).
#[test]
fn ddl_refuses_a_legacy_dictionary() {
    let dir = temp_fixture_dir("ddl-legacy");
    let dict = dir.join("data-dict.yaml");
    std::fs::write(
        &dict,
        indoc::indoc! {r#"
            $version: 0.1.0
            $learn_more: https://github.com/pjc-wspace/dbdict
            tables:
              - name: trades
                columns:
                  - name: qty
                    type: number
                    examples: [1, 2, 3]
        "#},
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_dbdict"))
        .arg("ddl")
        .arg(&dict)
        .output()
        .expect("failed to run dbdict");
    assert!(!output.status.success(), "legacy must refuse");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot generate DDL from a legacy"),
        "got:\n{stderr}"
    );
}
