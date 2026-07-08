# summary: dummy-data generator

started: 2026-07-07T14:10
closed: 2026-07-09T11:48:56+12:00

## goal

Add a dummy-data generator: the first consumer that *interprets* dbdict's
type system rather than round-tripping it. Generate a DuckDB database from a
rich `dbdict.yaml` whose values satisfy every declared constraint (D01–D05)
**by construction**, using the existing `validate-data` as a built-in oracle —
generated output must always pass it. Split into a backend-generic planning
crate and a DuckDB-specific rendering crate, wired as a CLI subcommand, with
`--rows`/`--seed` config, SQL export, and reproducible seeded output.

## what was accomplished

All 6 phases + an inserted interphase, each verified and committed:

- **Phase 1 — DuckDB extensions declaration (core):** new `duckdb: extensions:`
  dictionary section parallel to `typedef:`; spec checks S19/S20 and engine
  check M10 (declared extension must LOAD); LOAD-only (never INSTALL), wired
  into every connection dbdict opens.
- **Phase 2 — generator scaffolding + typed value generation:** two new crates
  (`dbdict-dummy-data`, `dbdict-dummy-data-duckdb`); paren/quote-aware type
  parser over the full canonical DuckDB surface; `nth(ty, i)` value generation
  — injective always, monotone for orderable types — with per-type `capacity`.
- **Phase 3 — generation plan:** topological table order (FK dependency,
  cycle-refusing), per-table row counts, per-column *roles* (indexed-unique, FK
  draw, plain fill), equality-join cardinality analysis; every unsupported
  shape refused with a descriptive error.
- **Phase 4 — end-to-end generation + oracle:** `generate` → SQL script (DDL +
  INSERTs) → `write_db`; deterministic seeded values via an inline FNV-1a +
  splitmix64 mix; integration tests prove D01–D04 + equality D05 by
  construction across the rich-type surface.
- **Interphase — code-review catch-up + capacity fixes:** a high-effort
  multi-agent review of everything since the last review found 10 findings;
  capacity fixes (BigInt/Interval/Union/Varchar) + an upfront unique-capacity
  refusal. Established the standing mandate to run `/code-review` at every
  phase boundary.
- **Phase 5 — D05 range joins + one-to-one:** slot arithmetic (stride 3:
  `nth(3k)`/`nth(3k+2)` bounds, `nth(3k+1)` probe), rel-salted owner draws, a
  shared `JoinExpr` orientation helper consumed by both the validator and the
  planner, and (user-directed) an S06 relaxation — a range join's at-most-one
  guarantee is a data property D05 checks, not a static unique-column
  property. Review caught 3 real defects TDD missed.
- **Phase 6 — CLI subcommand + SQL export + docs:** `dbdict dummy` (`run_dummy`
  mirroring `run_ddl`) with `--out/--sql/--force`, `--rows/--rows-table/--seed`,
  and `--null-fraction` (default 0.10); README + `site/index.md` updated
  (dummy-data moved from "planned" to "shipping"). Phase-boundary review: 10
  findings, 9 fixed (temp+rename atomic write closing a `--force` data-loss
  window + stale-`.wal` removal, corrected doc claims, stronger test
  assertions, hardened `--rows-table` parsing), 1 deferred.

Final state: `cargo test --workspace` **414 passed / 0 failed**, clippy 0
warnings, `cargo fmt --check` clean. HEAD `87c4ba0` on branch `duckdb-source`.

## key decisions

- **Deterministic indexed generation** (`nth(seed, table, column, i)`):
  injective in `i`, monotone for orderable types — so uniqueness (D02/D03),
  FK correctness (D04), and range slots (D05) reduce to index arithmetic with
  no rejection sampling and no reading data back.
- **Value strategy = SQL INSERT literals**, executed via `Connection`, not the
  Appender API — directly satisfies the `--sql` export requirement and is
  readable when debugging; speed explicitly not a goal.
- **Total-over-behavior type coverage:** every canonical type string either
  generates or is refused with a descriptive error — never a panic or a
  silently-wrong value.
- **Layer split follows the data:** structural refusals (roles, cardinality)
  live in the backend-generic planner; type-level refusals (orderable bounds,
  capacity) live in the DuckDB renderer, which alone parses type strings.
- **Range-join slot stride 3, not 2:** stride 2 leaves no `nth` value strictly
  between bounds, so `Gt`/`Lt` would be unsatisfiable; stride 3 handles open
  and closed bounds uniformly.
- **S06 relaxed for range joins** (user-directed): a unique bound is neither
  necessary nor sufficient for the at-most-one guarantee; orientation moved to
  the shared `JoinExpr` helper, fixing latent case/self-join drift.
- **`--out` always explicit, never defaults to `source.file`; `--force`
  overwrites via write-to-temp-then-rename** so a failed run never destroys an
  existing database — the CLI owns the overwrite decision, the library owns the
  refuse-existing guard.
- **Standing process change:** run `/code-review` (agent, high effort) at every
  phase boundary — it caught real bugs TDD missed in the interphase, phase 5,
  and phase 6.

## insights captured

In `.claude-work/insights/` from this session:
- `20260707-1708-extension-loading-under-hardened-duckdb`
- `20260707-2105-plan-builder-mirrors-oracle-fails-at-plan-time`
- `20260707-2127-identity-fk-draws-and-exact-null-fraction`
- `20260708-0954-capacity-contracts-review-vs-tests`
- `20260708-1516-shared-orientation-helper-kills-planner-validator-drift`
- `20260708-1612-range-claims-map-and-copy-uniqueness`
- `20260709-0958-agent-review-catches-what-tdd-misses`
- `20260709-1128-claims-and-tests-that-dont-verify-what-they-claim`

## follow-ups (recorded in impl.md, not built)

- Extract the shared `load_lowered_or_exit` scaffold (run_resolve/run_ddl/
  run_dummy) — 3× duplication.
- Make `Generated.script` self-contained (fold in extension `LOAD`s) so `--sql`
  is a complete standalone reproduction.
- Fix the pre-existing `ddl_refuses_a_legacy_dictionary` false-positive test.
- `--install-extensions` (network INSTALL opt-in) — LOAD-only this session.
- Minor polish: a `SLOT_STRIDE` constant for the literal `3`; `refuse`-closure
  dedups in generate.rs/plan.rs.
