//! Integration tests for the `validate` entry point.
//!
//! Each test points at a YAML fixture under `tests/fixtures/{valid,invalid}/`.
//! The fixtures double as runnable inputs for the CLI:
//!
//!     cargo run -p data-dict-cli -- validate-schema \
//!         crates/data-dict/tests/fixtures/invalid/enum-without-values.yaml
//!
//! When adding a new rule, prefer adding a fixture file (with a one-line
//! `# expected: ...` header) and a one-line test here over inline YAML.

use std::path::PathBuf;

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(rel)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn assert_valid(path: PathBuf) {
    if let Err(e) = data_dict::validate(&path) {
        panic!("expected {} to validate, but:\n{e}", path.display());
    }
}

/// Validate a fixture that must fail, returning the rendered diagnostic with
/// machine-specific noise stripped so it can be snapshotted.
///
/// The diagnostic carries two unstable bits: terminal styling (ANSI color
/// escapes and OSC-8 hyperlinks, the latter embedding an absolute `file://`
/// URL) and the absolute on-disk path of the fixture. We strip the escapes and
/// rewrite the path to its `tests/fixtures/`-relative form.
fn invalid_diagnostic(rel: &str) -> String {
    let path = fixture(rel);
    let diagnostic = match data_dict::validate(&path) {
        Ok(()) => panic!("expected {rel} to fail validation, but it passed"),
        Err(e) => e.to_string(),
    };
    let fixtures_root = format!(
        "{}/",
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .display()
    );
    strip_terminal_escapes(&diagnostic).replace(&fixtures_root, "")
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

// --- valid fixtures ------------------------------------------------------

#[test]
fn minimal() {
    assert_valid(fixture("valid/minimal.yaml"));
}

#[test]
fn example_foodbank() {
    assert_valid(workspace_root().join("examples/foodbank.yaml"));
}

#[test]
fn example_otters() {
    assert_valid(workspace_root().join("examples/otters.yaml"));
}

#[test]
fn example_elevators() {
    assert_valid(workspace_root().join("examples/elevators.yaml"));
}

// --- invalid fixtures ----------------------------------------------------

// Each invalid fixture snapshots its rendered diagnostic, so the test guards
// both that validation fails *and* that the user-facing message stays stable.
// Regenerate snapshots after an intentional message change with:
//
//     INSTA_UPDATE=always cargo test -p data-dict

#[test]
fn missing_version() {
    insta::assert_snapshot!(invalid_diagnostic("invalid/missing-version.yaml"));
}

#[test]
fn unknown_top_level_key() {
    insta::assert_snapshot!(invalid_diagnostic("invalid/unknown-top-level-key.yaml"));
}

#[test]
fn enum_without_values() {
    insta::assert_snapshot!(invalid_diagnostic("invalid/enum-without-values.yaml"));
}

#[test]
fn range_on_string_type() {
    insta::assert_snapshot!(invalid_diagnostic("invalid/range-on-string-type.yaml"));
}

#[test]
fn bad_cardinality() {
    insta::assert_snapshot!(invalid_diagnostic("invalid/bad-cardinality.yaml"));
}

#[test]
fn non_string_glossary_value() {
    insta::assert_snapshot!(invalid_diagnostic("invalid/non-string-glossary-value.yaml"));
}
