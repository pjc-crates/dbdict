# implementation: cleanup follow-ups (post dummy-data-generator)

Ordered lowest-risk → highest so confidence compounds: a no-behavior rename, a
test-only fix, a behavior-preserving refactor, then the one real behavior change.
TDD per item (failing test first where there's behavior to pin). `/code-review`
(agent, high effort) at the phase boundary — one pass over the whole diff at the
end, per the standing mandate.

## phases

### phase 1: name the range-join stride (item 4) — DONE 2026-07-10T10:57:54+12:00
No behavior change — a readability pass, done first to warm up.
- also (plan-vs-reality): the stride arithmetic lives in the DuckDB **renderer**
  (`dbdict-dummy-data-duckdb/src/generate.rs`), not `plan.rs` — `plan.rs` only
  carries the *rationale* in role doc-comments. Const placed in generate.rs.
- [x] added `const SLOT_STRIDE: u64 = 3;` after the imports in
      `crates/dbdict-dummy-data-duckdb/src/generate.rs` with the "minimum stride
      that leaves an interior index" rationale as its doc comment
- [x] replaced all four literal-`3` sites with `SLOT_STRIDE`: the two capacity
      checks (`saturating_mul`/`checked_mul`, generate.rs:125/468) and the two
      slot-index computations (`SLOT_STRIDE*row+offset` bound, `SLOT_STRIDE*k+1`
      probe, generate.rs:536/546); intra-slot offsets 0/1/2 kept literal (they're
      positions within a slot, not the stride itself)
- [x] refuse-closure dedup **skipped** (decision honored): only one `refuse`
      closure exists (plan.rs:467); generate.rs uses inline `.map_err` with a
      different error type — factoring would force a shared type across crates
- **verify (automated):** `cargo test --workspace` green (all D05 range tests:
  `distinct_ranges_do_not_overmatch`, `overlapping_ranges_overmatch`,
  `s06_*_range_join_needs_no_unique`, `validate_data_rich_reports_d01_through_d05`);
  byte-identical since `SLOT_STRIDE == 3`. clippy 0 warnings, fmt clean
- **verify (manual):** diff reviewed — every replaced `3` was a slot stride

### phase 2: fix the false-positive legacy DDL test (item 3) — DONE 2026-07-10T11:00:51+12:00
Test-only change, but it exposes whether the real refusal path works.
- [x] made the fixture spec-valid — added `examples: [1, 2, 3]` to the `number`
      column so it clears S07 and actually reaches `DdlError::LegacyUnsupported`
      (mirrors `dummy_refuses_a_legacy_dictionary`, cli.rs:560)
- [x] changed the assertion `contains("legacy")` → `contains("cannot generate DDL
      from a legacy")` — the bare word matched the temp-dir path
      `dbdict-cli-ddl-legacy-*`; the phrase can only come from the refusal message
      (`crates/dbdict-ddl/src/lib.rs:46`). rewrote the doc comment to record both
      traps.
- [x] **proof (i)** — reverted the fixture to no-`examples` and confirmed the test
      FAILS (dies at S07, not the generator): needle absent, so the fixture change
      was genuinely required. restored.
- [x] **proof (ii)** — bypassed `DdlError::LegacyUnsupported` (`if false && …` in
      ddl/lib.rs:109) and confirmed the test FAILS at the *message* assertion: the
      legacy dict then hits `ScriptFailed` (a different error), so the test pins
      the specific refusal, not merely a nonzero exit. restored via `git checkout`
      (file was clean at HEAD).
- **verify (automated):** `ddl_refuses_a_legacy_dictionary` passes; both failure
  probes confirmed. `git status` clean except the intended cli.rs change.
- **verify (manual):** the asserted phrase cannot appear in the temp-dir name

### phase 3: extract the shared CLI load helper (item 1)
Behavior-preserving refactor — covered by existing CLI tests, no assertion edits.
- [ ] add a helper in `crates/dbdict-cli/src/main.rs` — e.g.
      `fn load_lowered_or_exit(dict: Option<PathBuf>) -> Result<(Problems, DataDict), ExitCode>`
      folding: `resolve_dict_path` → `load_and_lower` → render load-errors (→
      `Err(FAILURE)`) → render warnings on the Ok path (→ `Ok((problems, dict))`)
- [ ] rewrite `run_resolve` (main.rs:301), `run_ddl` (main.rs:336), `run_dummy`
      (main.rs:406) to call it; keep each command's *own* post-load logic
      (typedef expand / ddl generate / dummy generate+write) in place
- [ ] preserve exact stderr ordering and exit codes — warnings still print on the
      Ok path before the command's own output
- **verify (automated):** `cargo test -p dbdict-cli` fully green with **zero**
  assertion changes (the whole point: behavior identical)
- **verify (manual):** diff shows the three functions shrank to the helper call +
  their unique tail; no message string changed

### phase 4: make `--sql` a self-contained reproduction (item 2)
The one real behavior change. Design (confirmed in goal): emit `LOAD name;` into
`script` at generation time; simplify `write_db`; drop the private `extensions`
field.
- [ ] TDD: add a test with a dict declaring a **non-autoloading** extension (not
      `json`, which autoloads — that masked the bug). **Round-trip assertion**
      (decided): execute the exported `--sql` script *alone* on a fresh
      `Connection` (no separate LOAD, no `write_db`) and assert the tables + rows
      are present — proving self-containment for real. Write it failing first.
- [ ] in `crates/dbdict-dummy-data-duckdb/src/generate.rs`, move the untrusted
      extension-name charset check (currently `write_db`, generate.rs:176–187) to
      generation time, and prepend validated `LOAD name;` lines to `script`
- [ ] drop the private `extensions` field (generate.rs:159) and its populate at
      generate.rs:312; simplify `write_db` (generate.rs:167) to open + run script
- [ ] flip the now-false comments/claims: `run_dummy`'s `--sql` comment
      (main.rs:459–461, "it is NOT a fully self-contained script") and the
      README / `site/index.md` line describing `--sql`
- [ ] grep for other prose asserting `--sql` completeness and correct each
- **verify (automated):** new test passes; `cargo test --workspace` green
- **verify (manual):** actually run `dbdict dummy` with `--sql` on a dict
  declaring a non-autoloading extension, open the exported `.sql` in a fresh
  DuckDB (or re-run just the script), confirm it builds the DB with no extra
  setup — verify the artifact, don't assert it in prose (insight
  20260709-1128)

### session close
- [ ] `/code-review` (agent, high effort) over the full session diff; address or
      explicitly defer each finding
- [ ] `cargo test --workspace` + `cargo clippy --workspace` (0 warnings) +
      `cargo fmt --check` all clean
- [ ] `/ws close`

## decisions (resolved at plan time)
- Phase 1 `refuse`-closure dedup: **decide at the code** — do the `SLOT_STRIDE`
  rename for sure; include the dedup only if it clearly reads cleaner in situ,
  else skip (don't force a shared type/import).
- Phase 4 verification: **round-trip** — re-execute the exported `.sql` alone on a
  fresh connection and assert the DB is reproduced (not just a text `contains`).
