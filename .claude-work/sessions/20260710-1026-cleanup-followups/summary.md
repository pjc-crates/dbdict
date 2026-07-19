# summary: cleanup follow-ups (post dummy-data-generator)

started: 2026-07-10T10:26
closed: 2026-07-15T12:26:26+12:00

## goal

Clear the four small follow-ups the dummy-data-generator session recorded rather
than built — two correctness-debt items surfaced by that session's review (a
false `--sql` self-containment claim; a false-positive legacy test) and two
readability items (3× CLI boilerplate; an unnamed stride literal). Pay down the
debt while the code was fresh, before the next model consumer (codegen) builds on
this surface. `--install-extensions` explicitly out of scope.

## what was accomplished

Four phases + a session-close code review, each verified and committed:

- **Phase 1 — `SLOT_STRIDE` (`dfdc184`):** named the range-join stride literal `3`
  (four sites in the DuckDB renderer, not `plan.rs` as the plan assumed — the
  arithmetic lives where roles become indices). Pure rename; D05 range-join tests
  byte-identical.
- **Phase 2 — legacy test false-positive (`e1a4f23`):** `ddl_refuses_a_legacy_dictionary`
  passed for the wrong reason — `contains("legacy")` matched the temp-dir path, and
  the fixture died at S07 before reaching the generator's refusal. Made the fixture
  spec-valid (`examples:`) and asserted the distinctive message phrase. Proved it
  fails both against the old fixture and with the refusal bypassed.
- **Phase 3 — `load_lowered_or_exit` (`5bdee96`):** extracted the resolve→load→
  render-warnings block duplicated across `run_resolve`/`run_ddl`/`run_dummy` into
  one helper returning `Result<DataDict, ExitCode>` (the `Err(code)` early-exit
  idiom). Behavior identical — CLI tests green with zero assertion changes.
- **Phase 4 — self-contained `--sql` (`45cf26a`):** folded declared extension
  `LOAD`s into `Generated.script` at generate() time (leading the DDL), simplified
  `write_db`, dropped the private `extensions` field. The `--sql` export is now a
  standalone script. New round-trip test executes it on a fresh connection.
- **Session-close review (`2c07131`):** high-effort review, 8 finder angles.
  Refactors traced clean. Fixed the accuracy findings — most importantly the
  `--sql` self-containment claim was over-general (the script `LOAD`s but never
  `INSTALL`s, so it holds only for bundled/autoloadable extensions), corrected in
  README + code docs after verifying against DuckDB's extension docs.

Final state: `cargo test --workspace` **415 passed / 0 failed**, clippy 0
warnings, `cargo fmt` clean. HEAD `2c07131` on branch `duckdb-source`.

## key decisions

- **Stride offsets left literal, not parametric:** `SLOT_STRIDE` names the stride
  factor; the intra-slot offsets (0/1/2) stay literal with a guard comment, rather
  than re-derived as `SLOT_STRIDE-1` — that would couple them to arithmetic only
  coincidentally true at stride 3. Naming a constant, not re-expressing semantics.
- **Helper renders warnings and returns `DataDict`:** callers never used the
  `ProblemSet` after the warning render, so the helper renders internally and hands
  back just the dict — simpler than the planned `(ProblemSet, DataDict)`.
- **`--sql` fold, not separate LOADs:** extension `LOAD`s belong *in* the script so
  the export is standalone; `write_db` just runs it. Accepted the loss of the
  per-`LOAD` "failed:" message framing (DuckDB's own error still names the
  extension) rather than reintroduce the separation the fold removed.
- **Text assertion is the real guard, not the round-trip:** json is statically
  linked (`bundled,json`), so no connection setting can force the explicit `LOAD` —
  proven empirically when a probe refuted the initial "disable autoload" fix. The
  `contains("LOAD json;")` text check is the autoload-independent regression guard;
  the round-trip proves executability only.
- **Charset-rule dedup deferred (F3):** the extension-name guard is now in 3 crates;
  a proper hoist to `dbdict` core is beyond this cleanup's scope. Recorded.

## insights captured

New this session (★ Insight blocks auto-scanned at close):
- Disabling DuckDB extension autoload does **not** gate a statically-linked
  (`bundled`) extension — its type is always available, so a round-trip on the only
  bundled extension (json) can't prove an explicit `LOAD` is required; a text/AST
  assertion must carry that. An unverified premise about this was caught by a probe.
- `git checkout <file>` restores HEAD, discarding uncommitted edits — reinforced
  the standing [[probe-restore-uncommitted]] memory (violated it mid-review; had to
  re-apply lost doc edits).

## follow-ups (recorded, not built)

- Hoist the extension-name charset rule to one shared validator in `dbdict` core
  (currently S19 + `native.rs::safe_extension_name` + `is_safe_extension_name`).
- `--install-extensions` (network `INSTALL` opt-in) — carried from the prior session.
