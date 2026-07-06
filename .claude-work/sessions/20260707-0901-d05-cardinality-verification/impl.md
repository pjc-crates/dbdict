# implementation: D05 cardinality verification

## phases

### phase 1: spec + core check + seam method + backend — DONE 2026-07-07T09:53:12+12:00
- [x] spec D05 in site/validation.md first (error, rich only): the data
      violates a declared `cardinality` — a row matches more than one row
      on a declared "one" side when the join is evaluated. Document: the
      direction table (`many-to-one`: each left row ≤1 right match;
      `one-to-many`: each right row ≤1 left match; `one-to-one`: both
      directions, one problem each); zero matches never violate
      (cardinality bounds multiplicity, not totality — D04 owns fk
      totality); NULL join columns match nothing under SQL comparison
      semantics, so they pass; count = rows on the probing side matching
      more than one row; why equality joins are still measured despite
      S06+D02/D03 (relationship-span diagnostic; range joins get their
      only coverage; D02/D05 double-report accepted)
- [x] orientation normalization in core (crates/dbdict/src/rich.rs):
      resolve each checked direction to (probe table, other table,
      oriented conjuncts) with db-side name spellings. Probing from the
      right side flips each conjunct: `Eq↔Eq`, `Ge↔Le`, `Gt↔Lt` — so the
      backend always answers one question, "how many probe rows match >1
      other row". `many-to-one` → probe left; `one-to-many` → probe
      right; `one-to-one` → two calls (left probe, right probe)
- also: conjuncts are *canonicalized per-conjunct* before probe
      orientation (a conjunct written right-to-left is mirrored), rather
      than assuming all conjuncts share the first's orientation the way
      S06 does; self-joins are always canonical, so their orientation is
      positional
- also: under `--table`, a relationship is in scope if it touches the
      selected table on either side
- [x] fifth `DuckdbBackend` seam method (crates/dbdict/src/rich.rs):
      `count_overmatched_rows(db_file, probe_table, other_table, conjuncts: &[OrientedConjunct]) -> Result<usize, String>`
      where `OrientedConjunct` is a small core struct
      `{ probe_column: String, op: JoinOp, other_column: String }`
      (db-side spellings, `JoinOp` reused from join_expr)

> decision (goal.md): the seam survives a fifth method but its shape
> stretches — D05 is the first check not describable as (table, column)
> pairs, so conjuncts cross the seam *as data* (columns + operator), not
> as SQL strings. Core stays SQL-free; the backend renders `JoinOp` to
> an operator. Re-check at the sixth method as tradition demands.

- [x] implement in crates/dbdict-duckdb/src/native.rs (reuse
      `quote_ident`, `open_read_only`, `query_count`): correlated-count
      shape —
      `SELECT count(*) FROM probe p WHERE (SELECT count(*) FROM other o WHERE p.c1 <op> o.d1 AND …) > 1`
      — aliases make self-joins work; render `JoinOp` → `= >= <= > <`;
      re-export at the crate root next to the other query functions
- also: defensive guard — an empty conjunct list returns `Err` rather
      than rendering invalid SQL (the core never sends one)
- [x] new `ProblemKind` variant `CardinalityViolation { count }` → code
      D05, Level::Data; message names the declared cardinality and the
      over-matched direction (dict spellings); anchored at the
      relationship's join text + cardinality spans (the S06 two-span
      pattern — relationships have no column constraint span)
- [x] extend the data pass in crates/dbdict/src/rich.rs: after the
      per-table loop, iterate `dict.relationships` — skip a relationship
      if either table is absent from the db (M06 already reported) or
      any join column is absent (M02 already reported), the D04 skip
      pattern; query failures report M05-shaped `UnreadableSource`
      (anchored at the relationship's join text)
- [x] tests, red first:
      - core fake-backend (crates/dbdict/tests/rich_data.rs): fake gains
        canned `overmatch_counts` keyed by probe direction + call log;
        `many-to-one` violation → one D05 with count, message names
        direction; `one-to-many` probes the right side (assert call
        shape); `one-to-one` makes two calls and a single violating
        direction yields exactly one problem; op flip asserted on a
        range join probed from the right; zero relationships → no
        calls; absent table / absent join column → no query (M06/M02
        present); query failure → UnreadableSource
      - real duckdb (crates/dbdict-duckdb/tests/data_queries.rs):
        overlapping date ranges violate `many-to-one` (the motivating
        gap); equality duplicate on the "one" side counted; distinct
        ranges pass; NULL join column rows match nothing and pass;
        zero-match rows pass; self-join; hostile-name quoting on both
        tables; case-insensitive identifiers; multi-conjunct (equality +
        two range conjuncts) evaluated together
- **verify:** `cargo test --workspace` green with new tests demonstrably
  red before implementation; `cargo clippy --workspace --all-targets` +
  `cargo fmt --check` clean — PASSED (290 passed / 0 failed, was 274:
  7 new fake-backend tests, 9 new real-duckdb tests incl. the
  overlapping-ranges motivating case, op-flip lock via one-to-one's
  second probe, NULL-join-column pass, self-join, and both-tables
  quoting)

### phase 2: CLI e2e + docs
- [ ] extend the rich-data CLI e2e fixtures (crates/dbdict-cli/tests/cli.rs):
      seeded fixture gains a `periods` table with *overlapping ranges*
      and a `many-to-one` range relationship → snapshot shows
      D01+D02+D03+D04+D05, D05 anchored at the relationship (test
      renamed to `..._d01_through_d05`; old snapshot deleted with the
      rename; .snap.new reviewed before accepting); clean fixture gains
      the same shape with non-overlapping ranges → still exits 0
- [ ] README.md: validate-data bullet mentions cardinality violations
      (D05) alongside D01–D04
- [ ] site/spec.md: the relationships section's `cardinality` bullet
      points at D05 in validation.md (the D03/D04 cross-reference
      pattern)
- **verify:** `cargo test --workspace` green; snapshots reviewed before
  accepting; seeded fixture exits 1 reporting D01–D05, clean fixture
  exits 0; `cargo clippy --workspace --all-targets` + `cargo fmt --check`
  clean
