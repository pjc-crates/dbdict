//! Integration tests for the metadata level on rich (0.2.0) documents: the
//! duckdb round-trip comparison, driven through a fake [`DuckdbBackend`].
//!
//! The fake returns canned scratch-instantiation results and database schemas,
//! so these tests pin down the *diff* logic (which problems, at which spans)
//! without a duckdb build. The real backend is `dbdict-duckdb`, proven by its
//! own round-trip tests.

mod common;
use common::{temp_dir, write_yaml};

use std::path::{Path, PathBuf};

use dbdict::model::DataDict;
use dbdict::rich::{DuckdbBackend, InstantiateFailure, Instantiated, TableSchema, TypeCategory};
use dbdict::{Problem, ProblemKind, Status, validate_meta};
use indoc::indoc;

/// Classification for the canonical spellings these fixtures use — just
/// enough of the real backend's classifier for the tests. This deliberately
/// covers only a handful of spellings; it is not the drift guard for the real
/// classifier (that is `dbdict-duckdb`'s `classify.rs`, which pins every arm
/// against live `DESCRIBE` output). These core tests exercise the *diff logic
/// given a classification*, so a divergence here can't mask a real-backend bug.
fn fixture_classify(canonical_type: &str) -> TypeCategory {
    match canonical_type {
        "BOOLEAN" => TypeCategory::Boolean,
        "DATE" => TypeCategory::Date,
        "TIMESTAMP" => TypeCategory::Timestamp,
        "TIMESTAMP WITH TIME ZONE" => TypeCategory::TimestampTz,
        t if t.starts_with("ENUM(") => TypeCategory::Enum,
        t if t.starts_with("DECIMAL(") || t == "BIGINT" => TypeCategory::Numeric,
        _ => TypeCategory::Other,
    }
}

/// a `(column, canonical type)` pair, as `DESCRIBE` would report it
fn col(name: &str, canonical_type: &str) -> (String, String) {
    (name.to_string(), canonical_type.to_string())
}

/// A canned backend: whatever the dictionary says, `instantiate` returns
/// `instantiated` and `read_schema` returns a clone of `db`.
struct FakeDuckdb {
    instantiated: Instantiated,
    db: Result<Vec<TableSchema>, String>,
}

impl DuckdbBackend for FakeDuckdb {
    fn instantiate(&self, _dict: &DataDict) -> Instantiated {
        self.instantiated.clone()
    }

    fn read_schema(&self, _db_file: &Path) -> Result<Vec<TableSchema>, String> {
        self.db.clone()
    }

    fn classify(&self, canonical_type: &str) -> TypeCategory {
        fixture_classify(canonical_type)
    }

    // the metadata level never queries values — reaching either is a bug
    fn count_nulls(&self, _db_file: &Path, _table: &str, _column: &str) -> Result<usize, String> {
        unreachable!("validate_meta must not run data queries")
    }

    fn count_duplicate_keys(
        &self,
        _db_file: &Path,
        _table: &str,
        _key_columns: &[String],
    ) -> Result<usize, String> {
        unreachable!("validate_meta must not run data queries")
    }

    fn count_duplicate_values(
        &self,
        _db_file: &Path,
        _table: &str,
        _column: &str,
    ) -> Result<usize, String> {
        unreachable!("validate_meta must not run data queries")
    }
}

/// The standard one-table dictionary these tests validate: `trades` with a
/// native-typed `qty` and an alias-typed `price`, sourcing `warehouse.duckdb`.
fn write_trades_dict(dir: &Path) -> PathBuf {
    write_yaml(
        dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: http://data-dict.tidyverse.org/
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
}

/// the scratch-side canonicalization of the `trades` dictionary table
fn trades_expected() -> Vec<(String, String)> {
    vec![col("qty", "BIGINT"), col("price", "DECIMAL(12,2)")]
}

#[test]
fn type_mismatch_is_m01_with_canonical_types() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    // the dictionary's `price` canonicalizes to DECIMAL(12,2), but the
    // database column is DECIMAL(18,4): an exact-string M01 mismatch
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![trades_expected()],
            failures: Vec::new(),
        },
        db: Ok(vec![TableSchema {
            name: "trades".to_string(),
            columns: vec![col("qty", "BIGINT"), col("price", "DECIMAL(18,4)")],
        }]),
    };

    let problems = validate_meta(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), kind: ProblemKind::TypeMismatch { declared, actual }, .. }]
                if *code == "M01" && declared == "DECIMAL(12,2)" && actual == "DECIMAL(18,4)"
        ),
        "got {:?}",
        problems.items
    );
    // the message names both canonical spellings: the dictionary side is an
    // alias (`money`), so the reader needs its expansion stated
    let message = &problems.items[0].message;
    assert!(
        message.contains("DECIMAL(12,2)") && message.contains("DECIMAL(18,4)"),
        "got {message:?}"
    );
}

#[test]
fn column_missing_from_database_is_m02() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    // the database table exists but has dropped the `price` column
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![trades_expected()],
            failures: Vec::new(),
        },
        db: Ok(vec![TableSchema {
            name: "trades".to_string(),
            columns: vec![col("qty", "BIGINT")],
        }]),
    };

    let problems = validate_meta(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), kind: ProblemKind::MissingInData, .. }] if *code == "M02"
        ),
        "got {:?}",
        problems.items
    );
}

#[test]
fn undocumented_database_column_is_m03_warning() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    // the database has a `venue` column the dictionary does not describe
    let mut columns = trades_expected();
    columns.push(col("venue", "VARCHAR"));
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![trades_expected()],
            failures: Vec::new(),
        },
        db: Ok(vec![TableSchema {
            name: "trades".to_string(),
            columns,
        }]),
    };

    let problems = validate_meta(&yaml, None, &backend);
    assert_eq!(
        problems.status(),
        Status::Warning,
        "got {:?}",
        problems.items
    );
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), column: Some(column), kind: ProblemKind::ExtraInData { actual }, .. }]
                if *code == "M03" && column == "venue" && actual == "VARCHAR"
        ),
        "got {:?}",
        problems.items
    );
}

#[test]
fn missing_dict_source_is_m04() {
    let dir = temp_dir();
    // no `source:` — valid at the spec level, but meta has nothing to compare
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: http://data-dict.tidyverse.org/
            tables:
              - name: trades
                columns:
                  - name: qty
                    type: BIGINT
        "#},
    );
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![vec![col("qty", "BIGINT")]],
            failures: Vec::new(),
        },
        db: Ok(Vec::new()),
    };

    let problems = validate_meta(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), kind: ProblemKind::MissingSource, .. }] if *code == "M04"
        ),
        "got {:?}",
        problems.items
    );
}

#[test]
fn unreadable_database_is_m05() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    // the backend cannot open the database the dictionary names
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![trades_expected()],
            failures: Vec::new(),
        },
        db: Err("unable to open database file".to_string()),
    };

    let problems = validate_meta(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), kind: ProblemKind::UnreadableSource, .. }] if *code == "M05"
        ),
        "got {:?}",
        problems.items
    );
    assert!(
        problems.items[0]
            .message
            .contains("unable to open database file"),
        "duckdb's own reason must reach the user, got {:?}",
        problems.items[0].message
    );
}

#[test]
fn table_missing_from_database_is_m06() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    // the database exists but holds no `trades` relation at all
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![trades_expected()],
            failures: Vec::new(),
        },
        db: Ok(Vec::new()),
    };

    let problems = validate_meta(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), kind: ProblemKind::MissingTable, .. }] if *code == "M06"
        ),
        "got {:?}",
        problems.items
    );
}

#[test]
fn undocumented_database_table_is_m07_warning() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    // the database holds an extra `audit_log` relation the dictionary
    // does not describe
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![trades_expected()],
            failures: Vec::new(),
        },
        db: Ok(vec![
            TableSchema {
                name: "trades".to_string(),
                columns: trades_expected(),
            },
            TableSchema {
                name: "audit_log".to_string(),
                columns: vec![col("id", "BIGINT")],
            },
        ]),
    };

    let problems = validate_meta(&yaml, None, &backend);
    assert_eq!(
        problems.status(),
        Status::Warning,
        "got {:?}",
        problems.items
    );
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), kind: ProblemKind::ExtraTable, .. }] if *code == "M07"
        ),
        "got {:?}",
        problems.items
    );
    assert!(
        problems.items[0].message.contains("audit_log"),
        "the undocumented table must be named, got {:?}",
        problems.items[0].message
    );
}

#[test]
fn multiple_undocumented_tables_each_get_an_m07() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    // two undocumented tables: both must be reported (emission follows the
    // order read_schema returns, which the native backend sorts alphabetically
    // — pinned in dbdict-duckdb's read_schema test)
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![trades_expected()],
            failures: Vec::new(),
        },
        db: Ok(vec![
            TableSchema {
                name: "alpha".to_string(),
                columns: vec![col("id", "BIGINT")],
            },
            TableSchema {
                name: "trades".to_string(),
                columns: trades_expected(),
            },
            TableSchema {
                name: "zeta".to_string(),
                columns: vec![col("id", "BIGINT")],
            },
        ]),
    };

    let problems = validate_meta(&yaml, None, &backend);
    let m07: Vec<&str> = problems
        .items
        .iter()
        .filter(|p| p.code == Some("M07"))
        .map(|p| p.message.as_str())
        .collect();
    assert_eq!(m07.len(), 2, "got {:?}", problems.items);
    assert!(
        m07[0].contains("alpha") && m07[1].contains("zeta"),
        "got {m07:?}"
    );
}

#[test]
fn rejected_typedefs_are_m08_at_their_spans() {
    let dir = temp_dir();
    // `ref_a` and `ref_b` form a cycle: the fixpoint stalls with both left
    // over, and each reports duckdb's own error at its typedef
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: http://data-dict.tidyverse.org/
            typedef:
              ref_a: ref_b
              ref_b: ref_a
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                typedef:
                  bad_scoped: NO_SUCH_TYPE
                columns:
                  - name: qty
                    type: BIGINT
        "#},
    );
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![vec![col("qty", "BIGINT")]],
            failures: vec![
                InstantiateFailure::Typedef {
                    table: None,
                    index: 0,
                    error: "Type with name ref_b does not exist!".to_string(),
                },
                InstantiateFailure::Typedef {
                    table: None,
                    index: 1,
                    error: "Type with name ref_a does not exist!".to_string(),
                },
                InstantiateFailure::Typedef {
                    table: Some(0),
                    index: 0,
                    error: "Type with name NO_SUCH_TYPE does not exist!".to_string(),
                },
            ],
        },
        db: Ok(vec![TableSchema {
            name: "trades".to_string(),
            columns: vec![col("qty", "BIGINT")],
        }]),
    };

    let problems = validate_meta(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    let m08: Vec<_> = problems
        .items
        .iter()
        .filter(|p| p.code == Some("M08"))
        .collect();
    assert_eq!(m08.len(), 3, "got {:?}", problems.items);
    assert!(
        m08.iter()
            .all(|p| matches!(p.kind, ProblemKind::InvalidTypedef)),
        "got {:?}",
        problems.items
    );
    // duckdb's reason reaches the user, and each problem is span-located
    assert!(m08[0].message.contains("ref_b does not exist"));
    assert!(
        m08.iter().all(|p| p.location(&problems.source).is_some()),
        "each rejected typedef must point at its source"
    );
}

#[test]
fn rejected_column_type_is_m09_at_its_span() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    // `price`'s alias never instantiates; the scratch table was still built
    // (without that column), so the rest of the table is still diffed
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![vec![col("qty", "BIGINT")]],
            failures: vec![InstantiateFailure::Column {
                table: 0,
                column: 1,
                error: "Type with name money does not exist!".to_string(),
            }],
        },
        db: Ok(vec![TableSchema {
            name: "trades".to_string(),
            columns: trades_expected(),
        }]),
    };

    let problems = validate_meta(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Error);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem { code: Some(code), kind: ProblemKind::InvalidColumnType, .. }] if *code == "M09"
        ),
        "got {:?}",
        problems.items
    );
    assert!(
        problems.items[0].message.contains("money does not exist"),
        "duckdb's reason must reach the user, got {:?}",
        problems.items[0].message
    );
    assert!(
        problems.items[0].location(&problems.source).is_some(),
        "a rejected column type must point at its source"
    );
}

/// A two-table dictionary where only `trades` matches its database twin;
/// `orders` has a type mismatch and a rejected column type, and the database
/// holds an extra table.
fn two_table_fixture(dir: &Path) -> (PathBuf, FakeDuckdb) {
    let yaml = write_yaml(
        dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: http://data-dict.tidyverse.org/
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                columns:
                  - name: qty
                    type: BIGINT
              - name: orders
                columns:
                  - name: id
                    type: BIGINT
                  - name: status
                    type: NO_SUCH_TYPE
        "#},
    );
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![
                vec![col("qty", "BIGINT")],
                // `status` failed to instantiate, so only `id` was created
                vec![col("id", "BIGINT")],
            ],
            failures: vec![InstantiateFailure::Column {
                table: 1,
                column: 1,
                error: "Type with name NO_SUCH_TYPE does not exist!".to_string(),
            }],
        },
        db: Ok(vec![
            TableSchema {
                name: "trades".to_string(),
                columns: vec![col("qty", "BIGINT")],
            },
            TableSchema {
                name: "orders".to_string(),
                columns: vec![col("id", "VARCHAR"), col("status", "VARCHAR")],
            },
            TableSchema {
                name: "audit_log".to_string(),
                columns: vec![col("id", "BIGINT")],
            },
        ]),
    };
    (yaml, backend)
}

#[test]
fn table_flag_checks_only_that_table() {
    let dir = temp_dir();
    let (yaml, backend) = two_table_fixture(&dir);

    // `orders` mismatches (M01, M09) and `audit_log` is undocumented (M07),
    // but validating `trades` alone must report none of them
    let problems = validate_meta(&yaml, Some("trades"), &backend);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);

    // and validating `orders` alone still reports its own problems
    let problems = validate_meta(&yaml, Some("orders"), &backend);
    assert_eq!(problems.status(), Status::Error);
    let codes: Vec<_> = problems.items.iter().filter_map(|p| p.code).collect();
    assert!(
        codes.contains(&"M01") && codes.contains(&"M09") && !codes.contains(&"M07"),
        "got {codes:?}: {:?}",
        problems.items
    );
}

#[test]
fn unknown_table_flag_is_preflight_failure() {
    let dir = temp_dir();
    let (yaml, backend) = two_table_fixture(&dir);

    let problems = validate_meta(&yaml, Some("nope"), &backend);
    assert!(
        matches!(
            problems.items.as_slice(),
            [Problem {
                kind: ProblemKind::TableNotFound { .. },
                ..
            }]
        ),
        "got {:?}",
        problems.items
    );
}

#[test]
fn database_path_resolves_relative_to_the_dictionary() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    // a fake that only answers for the resolved path proves the dictionary's
    // relative `file` is joined onto the dictionary's own directory
    struct PathCheckingFake {
        expect: PathBuf,
    }
    impl DuckdbBackend for PathCheckingFake {
        fn instantiate(&self, _dict: &DataDict) -> Instantiated {
            Instantiated {
                tables: vec![trades_expected()],
                failures: Vec::new(),
            }
        }
        fn read_schema(&self, db_file: &Path) -> Result<Vec<TableSchema>, String> {
            if db_file != self.expect {
                return Err(format!(
                    "resolved to {} instead of {}",
                    db_file.display(),
                    self.expect.display()
                ));
            }
            Ok(vec![TableSchema {
                name: "trades".to_string(),
                columns: trades_expected(),
            }])
        }

        fn classify(&self, canonical_type: &str) -> TypeCategory {
            fixture_classify(canonical_type)
        }
        fn count_nulls(&self, _db: &Path, _table: &str, _col: &str) -> Result<usize, String> {
            unreachable!("validate_meta must not run data queries")
        }
        fn count_duplicate_keys(
            &self,
            _db: &Path,
            _table: &str,
            _keys: &[String],
        ) -> Result<usize, String> {
            unreachable!("validate_meta must not run data queries")
        }
        fn count_duplicate_values(
            &self,
            _db: &Path,
            _table: &str,
            _col: &str,
        ) -> Result<usize, String> {
            unreachable!("validate_meta must not run data queries")
        }
    }
    let backend = PathCheckingFake {
        expect: dir.join("warehouse.duckdb"),
    };

    let problems = validate_meta(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
}

#[test]
fn absolute_database_path_is_used_verbatim() {
    // an absolute `source.duckdb.file` must reach the backend unchanged — not
    // joined onto the dictionary's directory (Path::join keeps an absolute
    // right-hand side as-is, and the fake pins that)
    let dir = temp_dir();
    let abs = dir.join("elsewhere").join("warehouse.duckdb");
    let yaml = write_yaml(
        &dir,
        &format!(
            "$version: \"0.2.0\"\n\
             $learn_more: http://data-dict.tidyverse.org/\n\
             source:\n  duckdb:\n    file: {}\n\
             tables:\n  - name: trades\n    columns:\n      - name: qty\n        type: BIGINT\n",
            abs.display()
        ),
    );
    struct ExpectAbsolute {
        expect: PathBuf,
    }
    impl DuckdbBackend for ExpectAbsolute {
        fn instantiate(&self, _dict: &DataDict) -> Instantiated {
            Instantiated {
                tables: vec![vec![col("qty", "BIGINT")]],
                failures: Vec::new(),
            }
        }
        fn read_schema(&self, db_file: &Path) -> Result<Vec<TableSchema>, String> {
            assert_eq!(db_file, self.expect, "absolute path must be used verbatim");
            Ok(vec![TableSchema {
                name: "trades".to_string(),
                columns: vec![col("qty", "BIGINT")],
            }])
        }
        fn classify(&self, canonical_type: &str) -> TypeCategory {
            fixture_classify(canonical_type)
        }
        fn count_nulls(&self, _db: &Path, _table: &str, _col: &str) -> Result<usize, String> {
            unreachable!("validate_meta must not run data queries")
        }
        fn count_duplicate_keys(
            &self,
            _db: &Path,
            _table: &str,
            _keys: &[String],
        ) -> Result<usize, String> {
            unreachable!("validate_meta must not run data queries")
        }
        fn count_duplicate_values(
            &self,
            _db: &Path,
            _table: &str,
            _col: &str,
        ) -> Result<usize, String> {
            unreachable!("validate_meta must not run data queries")
        }
    }

    let problems = validate_meta(&yaml, None, &ExpectAbsolute { expect: abs });
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
}

#[test]
fn instantiation_failures_report_even_when_the_database_is_unreadable() {
    let dir = temp_dir();
    // a cyclic typedef (M08) AND an unopenable database (M05): the dict-side
    // coherence check runs before the source is read, so BOTH must surface —
    // the reader shouldn't have to open the db to learn the dict is incoherent
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: http://data-dict.tidyverse.org/
            typedef:
              broken: NO_SUCH_TYPE
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                columns:
                  - name: qty
                    type: BIGINT
        "#},
    );
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![vec![col("qty", "BIGINT")]],
            failures: vec![InstantiateFailure::Typedef {
                table: None,
                index: 0,
                error: "Type with name NO_SUCH_TYPE does not exist!".to_string(),
            }],
        },
        db: Err("unable to open database file".to_string()),
    };

    let problems = validate_meta(&yaml, None, &backend);
    let codes: Vec<&str> = problems.items.iter().filter_map(|p| p.code).collect();
    assert!(
        codes.contains(&"M08") && codes.contains(&"M05"),
        "expected both M08 and M05, got {codes:?}: {:?}",
        problems.items
    );
}

#[test]
fn untyped_column_makes_no_type_claim() {
    let dir = temp_dir();
    // `note` is documented without a `type:`: present in the database it is
    // neither type-checked nor undocumented
    let yaml = write_yaml(
        &dir,
        indoc! {r#"
            $version: "0.2.0"
            $learn_more: http://data-dict.tidyverse.org/
            source:
              duckdb:
                file: warehouse.duckdb
            tables:
              - name: trades
                columns:
                  - name: qty
                    type: BIGINT
                  - name: note
        "#},
    );
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            // the scratch table holds only the typed column
            tables: vec![vec![col("qty", "BIGINT")]],
            failures: Vec::new(),
        },
        db: Ok(vec![TableSchema {
            name: "trades".to_string(),
            columns: vec![col("qty", "BIGINT"), col("note", "VARCHAR")],
        }]),
    };

    let problems = validate_meta(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
}

/// Build a one-table dictionary around one column body (name `c`, plus the
/// caller's extra keys), with a backend whose database matches the scratch
/// side exactly — so every problem the tests see comes from the descriptive
/// keys, not the round-trip diff.
fn one_column_fixture(
    dir: &Path,
    canonical_type: &str,
    column_body: &str,
) -> (PathBuf, FakeDuckdb) {
    let column = column_body
        .trim_end()
        .lines()
        .map(|line| format!("        {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let yaml = write_yaml(
        dir,
        &format!(
            "$version: \"0.2.0\"\n\
             $learn_more: http://data-dict.tidyverse.org/\n\
             source:\n  duckdb:\n    file: warehouse.duckdb\n\
             tables:\n  - name: t\n    columns:\n      - name: c\n{column}\n"
        ),
    );
    let expected = vec![col("c", canonical_type)];
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![expected.clone()],
            failures: Vec::new(),
        },
        db: Ok(vec![TableSchema {
            name: "t".to_string(),
            columns: expected,
        }]),
    };
    (yaml, backend)
}

/// Run a one-column fixture and return the problem codes it raises.
fn codes_for(canonical_type: &str, column_body: &str) -> Vec<&'static str> {
    let dir = temp_dir();
    let (yaml, backend) = one_column_fixture(&dir, canonical_type, column_body);
    let problems = validate_meta(&yaml, None, &backend);
    problems.items.iter().filter_map(|p| p.code).collect()
}

#[test]
fn s07_rich_rejects_range_on_unorderable_types() {
    // a struct is not orderable; an enum lists categories, not bounds
    assert_eq!(
        codes_for(
            "STRUCT(city VARCHAR)",
            "type: STRUCT(city VARCHAR)\nrange: [1, 2]"
        ),
        vec!["S07"]
    );
    assert_eq!(
        codes_for(
            "ENUM('happy', 'sad')",
            "type: ENUM('happy', 'sad')\nrange: [1, 2]"
        ),
        vec!["S07"]
    );
    // orderable types may carry a range
    assert_eq!(
        codes_for("DECIMAL(12,2)", "type: DECIMAL(12, 2)\nrange: [0, 100]"),
        Vec::<&str>::new()
    );
    assert_eq!(
        codes_for("DATE", "type: DATE\nrange: [2020-01-01, 2024-12-31]"),
        Vec::<&str>::new()
    );
}

#[test]
fn s07_rich_rejects_any_representation_on_boolean() {
    assert_eq!(
        codes_for("BOOLEAN", "type: BOOLEAN\nvalues: [yes, no]"),
        vec!["S07"]
    );
    assert_eq!(
        codes_for("BOOLEAN", "type: BOOLEAN\nrange: [0, 1]"),
        vec!["S07"]
    );
    assert_eq!(
        codes_for("BOOLEAN", "type: BOOLEAN\nexamples: [true]"),
        vec!["S07"]
    );
}

#[test]
fn s07_rich_requires_nothing() {
    // the rich types carry no measure/id qualifier, so no representation can
    // be *required* — a bare column of any type is fine
    assert_eq!(codes_for("BIGINT", "type: BIGINT"), Vec::<&str>::new());
    assert_eq!(codes_for("VARCHAR", "type: VARCHAR"), Vec::<&str>::new());
    assert_eq!(
        codes_for("ENUM('happy', 'sad')", "type: ENUM('happy', 'sad')"),
        Vec::<&str>::new()
    );
    // `values` on a plain VARCHAR is a legitimate categorical column
    assert_eq!(
        codes_for("VARCHAR", "type: VARCHAR\nvalues: [M, F, U]"),
        Vec::<&str>::new()
    );
}

#[test]
fn s08_rich_allows_units_on_numerics_only() {
    assert_eq!(
        codes_for("DECIMAL(12,2)", "type: DECIMAL(12, 2)\nunits: USD"),
        Vec::<&str>::new()
    );
    assert_eq!(
        codes_for("VARCHAR", "type: VARCHAR\nunits: kg"),
        vec!["S08"]
    );
}

#[test]
fn s14_rich_allows_time_zone_on_timestamps_only() {
    assert_eq!(
        codes_for("TIMESTAMP", "type: TIMESTAMP\ntime_zone: UTC"),
        Vec::<&str>::new()
    );
    assert_eq!(
        codes_for(
            "TIMESTAMP WITH TIME ZONE",
            "type: TIMESTAMP WITH TIME ZONE\ntime_zone: UTC"
        ),
        Vec::<&str>::new()
    );
    assert_eq!(codes_for("DATE", "type: DATE\ntime_zone: UTC"), vec!["S14"]);
}

#[test]
fn s12_rich_checks_range_bound_types() {
    // a string bound on a numeric range
    assert_eq!(
        codes_for("DECIMAL(12,2)", "type: DECIMAL(12, 2)\nrange: [low, 100]"),
        vec!["S12"]
    );
    // a non-date bound on a date range
    assert_eq!(
        codes_for("DATE", "type: DATE\nrange: [soon, 2024-12-31]"),
        vec!["S12"]
    );
    // zoneless timestamps take naive bounds; zoned ones carry offsets
    assert_eq!(
        codes_for(
            "TIMESTAMP",
            "type: TIMESTAMP\nrange: [2024-01-01T00:00:00, 2024-12-31T23:59:59]"
        ),
        Vec::<&str>::new()
    );
    assert_eq!(
        codes_for(
            "TIMESTAMP WITH TIME ZONE",
            "type: TIMESTAMP WITH TIME ZONE\nrange: [2024-01-01T00:00:00, 2024-12-31T23:59:59]"
        ),
        vec!["S12", "S12"],
        "zoned timestamp bounds must carry an offset"
    );
    // an open bound is welcome on any orderable type
    assert_eq!(
        codes_for("DECIMAL(12,2)", "type: DECIMAL(12, 2)\nrange: [-.inf, 100]"),
        Vec::<&str>::new()
    );
}

#[test]
fn s13_rich_checks_range_order() {
    assert_eq!(
        codes_for("DECIMAL(12,2)", "type: DECIMAL(12, 2)\nrange: [100, 0]"),
        vec!["S13"]
    );
    assert_eq!(
        codes_for("DATE", "type: DATE\nrange: [2024-12-31, 2020-01-01]"),
        vec!["S13"]
    );
    assert_eq!(
        codes_for("DECIMAL(12,2)", "type: DECIMAL(12, 2)\nrange: [.inf, 0]"),
        vec!["S13"],
        "an infinite bound on the wrong end runs backwards"
    );
}

#[test]
fn rich_does_not_type_check_examples_or_values() {
    // deliberate gap: in rich mode only `range` bounds are type-checked (S12).
    // `examples` and `values` are illustrative/categorical documentation — a
    // duckdb column carries no coarse "is this an id or a measure" intent for
    // them to contradict, and the exact type round-trip (M01) already pins
    // correctness. so a type-mismatched example/value raises nothing.
    assert_eq!(
        codes_for(
            "DECIMAL(12,2)",
            "type: DECIMAL(12, 2)\nexamples: [not_a_number]"
        ),
        Vec::<&str>::new()
    );
    assert_eq!(
        codes_for("BIGINT", "type: BIGINT\nvalues: [alpha, beta]"),
        Vec::<&str>::new()
    );
}

#[test]
fn clean_match_validates_ok() {
    let dir = temp_dir();
    let yaml = write_trades_dict(&dir);
    // the database agrees with the dictionary exactly
    let backend = FakeDuckdb {
        instantiated: Instantiated {
            tables: vec![trades_expected()],
            failures: Vec::new(),
        },
        db: Ok(vec![TableSchema {
            name: "trades".to_string(),
            columns: trades_expected(),
        }]),
    };

    let problems = validate_meta(&yaml, None, &backend);
    assert_eq!(problems.status(), Status::Ok, "got {:?}", problems.items);
}
