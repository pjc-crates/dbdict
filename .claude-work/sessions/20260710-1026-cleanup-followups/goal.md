# cleanup follow-ups (post dummy-data-generator)

## problem

The dummy-data-generator session (closed HEAD `9df9316`) deliberately recorded a
handful of small follow-ups in its `impl.md`/`summary.md` rather than growing its
own scope. Two of them are correctness debt surfaced by the adversarial review,
not just polish:

- `dummy --sql` exports `Generated.script` (DDL + INSERTs) but the extension
  `LOAD`s run separately in `write_db`, so the exported script is **not** a
  complete, standalone reproduction — the doc/comment claim to the contrary was
  literally false for any dict declaring a non-autoloading extension.
- `ddl_refuses_a_legacy_dictionary` is a **false-positive test**: its
  `contains("legacy")` needle matches the temp-dir path, not the refusal message,
  and its `0.1.0` fixture dies at S07 spec-validation before ever reaching the
  generator's legacy refusal. It gives false confidence that the legacy-refusal
  path works.

The remaining two are readability/maintenance debt: 3× duplication of the CLI
load-and-lower boilerplate, and an unnamed `stride` literal `3`.

Clearing these now — while the code is fresh — keeps the closed session actually
closed and pays down debt before the next model consumer (codegen) is built on
top of this CLI/generator surface.

## success criteria

- **Item 1 — shared CLI helper:** `run_resolve`, `run_ddl`, `run_dummy` no longer
  duplicate the `resolve_dict_path` → `load_and_lower` → render-warnings block;
  they call one helper. Behavior (exit codes, stderr warnings/errors) unchanged —
  existing CLI tests still pass with no assertion changes.
- **Item 2 — self-contained `--sql`:** the exported script, run on a fresh DuckDB
  with no other setup, reproduces the same database (extensions `LOAD`ed by the
  script itself). The untrusted-name charset check still runs before any name is
  interpolated into SQL. A test declaring a non-autoloading extension proves the
  exported `.sql` is complete.
- **Item 3 — legacy test fixed:** the fixture is spec-valid at every layer above
  the generator (reaches the real legacy refusal), and the assertion matches a
  distinctive phrase from the actual refusal message — not something the fixture
  path/setup can accidentally contain. The test fails if the refusal regresses.
- **Item 4 — named stride:** the range-join slot stride literal `3` is a named
  constant with the "strictly-between edges" rationale at its definition; optional
  `refuse`-closure dedup in generate.rs/plan.rs if it reads cleaner.
- **Whole session:** `cargo test --workspace` green, `cargo clippy` 0 warnings,
  `cargo fmt --check` clean. `/code-review` (agent, high effort) run at the phase
  boundary per the standing mandate; findings addressed or explicitly deferred.

## scope

- in: items 1–4 above (CLI helper extraction, self-contained `--sql`, legacy test
  fix, `SLOT_STRIDE` constant + optional refuse-closure dedup).
- out: `--install-extensions` (network `INSTALL` opt-in) — a *feature* with
  network-safety and offline-testing concerns; its own future session.
- out: any change to generation semantics, type coverage, or constraint behavior
  (D01–D05) — this session is debt paydown, not new capability.

## constraints

- Rust-learner conventions: plain, explicit code; thorough "why" comments; no
  clever iterator/macro/lifetime tricks (project CLAUDE.md).
- Layer discipline holds: type-string parsing / charset validation stays in the
  DuckDB renderer crate; the backend-generic planner never parses type strings.
- Item 2's design call (recommended): validate + emit `LOAD name;` into `script`
  during `generate()`, then simplify `write_db` to just execute the script and
  drop the private `extensions` field. Confirm/veto during `/ws plan`.
- TDD per item (write the failing test first); `/code-review` at the boundary.
