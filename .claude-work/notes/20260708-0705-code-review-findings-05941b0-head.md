# code review findings — 05941b0..HEAD (46b37eb)

reviewed: 2026-07-08 (high effort: 8 finder angles → 25 candidates → 1-vote
verify → 20 survived, 5 refuted, 10 reported). last prior review was
2026-07-06 (duckdb-spec phase 4). scope: D01–D05 data checks, dbdict-ddl,
`duckdb: extensions:` feature, dummy-data crates (phases 1–4 of session
20260707-1410-dummy-data-generator).

status: **findings 1–5 FIXED 2026-07-08** (TDD, same day as the review —
see per-finding notes below; workspace tests + clippy + fmt green).
findings 6–10 and the cut items remain for a coming session.
line numbers are as of HEAD `46b37eb`; re-anchor if the files have moved.

## priority context

phase 5 (range-join slots) builds directly on the monotone/orderable
contracts in values.rs and on the D05 orientation logic in plan.rs. the
values.rs contract fixes (1, 2, 3, 5) and the capacity refusal (4) are
done; finding 9 (orientation helper) should be folded into phase 5's
design rather than fixed separately.

## reported findings (10)

### correctness — confirmed (ALL FIVE FIXED 2026-07-08)

1. **BIGINT capacity overflow — FIXED** — `dbdict-dummy-data-duckdb/src/values.rs:60`
   `capacity()` returns `u64::MAX` for BigInt but BIGINT is signed i64.
   plain-fill index = `mix(...) % capacity` (generate.rs:277) spans [0, 2^64),
   so ~half the rows render literals > i64::MAX → DuckDB "out of range for
   INT64" at INSERT (CLI-verified). siblings halve correctly (TinyInt=128,
   Integer=1<<31); BigInt is wrongly lumped with HugeInt.
   **fixed:** capacity = 1<<63 (HugeInt split out, stays u64::MAX); test
   `bigint_capacity_fits_the_signed_range` engine-inserts nth at cap-1.

2. **INTERVAL literal overflow — FIXED** — `values.rs:81`
   capacity `u64::MAX` but `INTERVAL {i} SECONDS` parser-errors for
   i >= 2^31 (CLI-verified at 2147483648 — the int32 literal syntax is the
   binding cap, not int64 micros). plain-fill INTERVAL fails on nearly every
   row. round-trip test only covers indices 0..20 so it never saw this.
   **fixed:** capacity = 1<<31; test `interval_capacity_fits_the_literal_syntax`
   engine-inserts nth at cap-1 (pins the 32-bit literal-syntax bound).

3. **UNION capacity vs nth() mismatch — FIXED** — `values.rs:87`
   capacity = min over ALL alternatives (lumped with Struct) but nth() only
   renders alternatives[0] (values.rs:216). unique UNION(BIGINT, BOOLEAN)
   exhausts spuriously at 2 rows via the i >= cap guard (values.rs:132).
   **fixed:** Union split from Struct in capacity(); = capacity(alternatives[0]).
   test `union_capacity_follows_the_first_alternative_only` (engine round-trips
   20 distinct values past a BOOLEAN second alternative).

4. **no plan-time capacity check for unique columns — FIXED** —
   `dbdict-dummy-data/src/plan.rs:167`
   plan() assigns Role::IndexedUnique with no capacity-vs-rows check (only
   the fk pigeonhole is checked, plan.rs:152), so BOOLEAN/TINYINT/ENUM PKs
   with rows > capacity fail at render as GenerateError::Value(Exhausted) —
   violating the documented invariant at plan.rs:100-101 ("refusals are
   always plan-time"). the unique-ENUM test (dummy-data-duckdb
   tests/generate.rs:312) currently asserts the render-time error — it must
   be updated to expect a plan-time refusal.
   **fixed (decision):** the check lives in generate() pre-render — an
   upfront loop over IndexedUnique columns returns the new
   `GenerateError::UniqueCapacityTooSmall {table, column, capacity, rows}`
   before any rendering (fk draws need no check: injective indices are
   bounded by the target's rows via the plan's pigeonhole). plan.rs
   doc amended honestly: "every refusal this crate can see happens here…
   type capacity is refused by the backend generator up front". the
   unique-ENUM test now asserts the new pre-render error.

5. **VARCHAR monotonicity break at 19→20 digits — FIXED** — `values.rs:162`
   `'v{i:019}'` but u64 needs 20 digits: nth(10^19) = 'v1000…' sorts before
   nth(10^19-1) = 'v9999…'. injectivity holds (different lengths), but the
   is_orderable contract (values.rs:115) is violated. latent today (range
   joins refused at plan.rs:205) but **phase 5's slot scheme relies on this
   exact contract for VARCHAR range bounds**.
   **fixed:** pads to 20 (`{i:020}`); test
   `varchar_stays_monotone_past_nineteen_digits` pins the boundary.

### correctness — plausible

6. **ddl script not self-contained w.r.t. extensions** —
   `dbdict-ddl/src/lib.rs:176`
   instantiate() LOADs declared extensions (native.rs:35) and write_db LOADs
   out-of-band (generate.rs:120-136), but dbdict-ddl emits no LOAD lines and
   its execute_and_describe self-check (native.rs:461-465) loads nothing.
   a dict whose types need a LOAD would validate cleanly yet fail `dbdict
   ddl` with ScriptFailed, and the exported .sql replayed standalone differs
   from write_db. latent on this build (json autoloads; parquet/icu add no
   types) but structural.
   **fix:** prepend `LOAD ext;` lines to the generated script and load in
   execute_and_describe — then write_db's out-of-band LOAD can go away
   (the script becomes the single source of truth, matching the module's
   "the script is the deliverable" stance).

7. **D04 double-reports duplicate fk→pk pairings** —
   `dbdict/src/rich.rs:689`
   foreign_key_targets (model.rs:85) pushes without dedup; two relationships
   may legally declare the same pairing (no S-check rejects it; plan.rs:271
   comment confirms). D04 loop pushes one OrphanedValues per target
   (rich.rs:714) and ProblemSet never dedups → two byte-identical problems.
   dummy-data's resolve_fk_targets dedups; D04 assumes the opposite.
   **fix:** dedup in foreign_key_targets itself (one authoritative
   contract), then drop the local dedup in plan.rs resolve_fk_targets.

8. **load_extensions default silently suppresses M10** —
   `dbdict/src/rich.rs:131`
   trait default returns Ok per name, so any backend not overriding it emits
   zero M10 UnloadableExtension diagnostics. only NativeDuckdb overrides
   (native.rs:253). was a deliberate phase-1 convenience (seven test fakes
   unchanged) but is a footgun for future backends/fakes.
   **fix option:** remove the default (make it required) and give the test
   fakes a one-line always-ok impl — explicit beats silent.

### reuse / conventions

9. **D05 orientation logic duplicated across crates** —
   `dbdict-dummy-data/src/plan.rs:219` vs `dbdict/src/rich.rs:407-472`
   first-conjunct orientation, cardinality→'one'-side mapping, and conjunct
   canonicalization (flip_op) derived independently; tied only by a comment
   (plan.rs:210-211). phase 5 extends exactly this logic for range joins —
   drift means generator emits data the validator rejects, with no compiler
   signal.
   **fix:** factor a model-level "oriented join" helper both crates consume.
   strongly consider doing this AS PART OF phase 5 rather than after.

10. **six uncited external-behavior claims in docs** — violates user-global
    CLAUDE.md sourcing rule ("quoted-and-cited … OR marked Inferred").
    - site/validation.md:15 — "Identifiers are matched case-insensitively,
      as DuckDB's are."
    - site/validation.md:42 — "DuckDB identifiers fold case"
    - site/validation.md:52 — "`LOAD` is idempotent"
    - site/validation.md:87 — SQL UNIQUE null semantics
    - site/validation.md:88 — SQL MATCH SIMPLE semantics
    - site/spec.md:403 — "NULLs are excluded, per SQL `UNIQUE` semantics."
    **fix:** fetch the official DuckDB/SQL doc for each, cite with link, or
    mark `Inferred:`. validation.md currently has zero external links.

## verified but cut at the 10-finding cap

efficiency (all CONFIRMED; repo prioritizes readability over speed, so
batch these as one cleanup pass):
- `dbdict-duckdb/src/native.rs:505` — every D01–D05 count query opens a
  fresh read-only connection (count_nulls 505 / dup_keys 523 / dup_values
  543 / orphans 569 / cardinality); ~T*C opens per validate-data run.
  fix: open once per run, pass the connection (or a query-session struct).
- `dbdict/src/rich.rs:354` — check_data re-reads the full schema that
  check_meta already read at rich.rs:264 and discarded. fix: have
  check_meta return/accept the schema.
- `dbdict-dummy-data-duckdb/src/generate.rs:155` — generate() scratch-
  instantiates the dict (1 global + T per-table DBs) then dbdict_ddl::
  generate does two more scratch builds (typedef_creation_order
  native.rs:447, execute_and_describe native.rs:462). fix: share/expose
  intermediate results, or accept and document.

duplication (CONFIRMED/PLAUSIBLE):
- `dbdict-dummy-data-duckdb/src/values.rs:254` — quote_ident_sql is a
  byte-identical copy of dbdict_duckdb::quote_ident (native.rs:646), which
  generate.rs:23 in the same crate already imports. fix: use the import.
- extension-name charset rule ([a-z0-9_]) in THREE places:
  validate_spec.rs:1089 (.chars()), native.rs:355 (.chars()),
  dummy-data-duckdb generate.rs:124 (.bytes()) — currently behaviorally
  equivalent, pure drift risk. fix: one shared predicate.
- identifier case-fold rule in three layers: validate_spec.rs:805 (s10_key),
  rich.rs:788 (names_eq), dbdict-ddl/src/lib.rs:201 (collect_typedefs).
  fix: model-level fold_ident/idents_eq helper.
- case-insensitive column lookup ×3 in rich.rs: db_column_in (515), the
  db_column closure (562), inlined in D04 (696-700). fix: call db_column_in.
- three near-identical span helpers rich.rs:753/767/776
  (requiredness/uniqueness/foreign_key_span) — one helper + predicate.
- plural + Ok(0)/Ok(n)/Err reporting scaffold repeated at rich.rs
  480/581/621/659/713 (weakest finding — bodies genuinely differ; only the
  plural helper is cleanly shareable).

latent, no reachable failure:
- `values.rs:191` GEOMETRY: stored f64 coords collide above 2^53
  (CLI-verified) contradicting the "distinct x per i" comment at :190 —
  literals stay distinct, nothing reachable needs geometry injectivity.
  fix the comment, or cap capacity at 2^53.

## refuted — do NOT re-report these

- S01 narrowed to equality conjuncts: DELIBERATE — fixture
  s01-fk-range-conjunct-only.yaml (`# expected: S01`), test
  validate_spec.rs:353, documented site/validation.md:33 ("Only `=` counts").
- `T[0]` zero-length array one-element literal: unreachable — DuckDB binder
  rejects `INTEGER[0]` ("ARRAY type size must be at least 1") before
  generate() ever maps the type.
- per-row rediscovery of per-column constants in generate.rs value loop:
  real but constant-factor over fixed dict-sized vectors — micro-cost under
  the repo's stated priorities.
- check_meta extension_names clone + zip: the same-order contract is
  documented on the trait (rich.rs:128-133) and the clone is forced by the
  deliberately duckdb-free `&[String]` seam.
- temp_db_file "identical ×3" claim: factually wrong (instantiate.rs has no
  such helper; the two real copies differ in prefix).

## suggested session shape (updated after the 2026-07-08 fixes)

1. ~~phase-5-blocking fixes~~ DONE (findings 1–5 above).
2. phase 5 proper, folding finding 9 (shared orientation helper) into its
   design.
3. findings 6, 7, 8 (extensions self-containment, D04 dedup, trait
   default) — independent, medium.
4. finding 10 (doc citations) + the cut efficiency/duplication batch as a
   cleanup phase or separate small session.
