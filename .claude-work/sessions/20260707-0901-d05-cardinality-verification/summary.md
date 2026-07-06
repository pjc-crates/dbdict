# summary: D05 cardinality verification

started: 2026-07-07 09:01
closed: 2026-07-07T10:28:03+12:00

## goal

`cardinality` was declared on every relationship but never verified
against the data. For equality joins the guarantees compose
(S06 + D02/D03), but for range/multi-conjunct joins S06 is deliberately
permissive and nothing prevented overlapping ranges from silently
violating a declared `many-to-one`. D05 (error, rich format only)
evaluates every relationship's join as declared and counts rows matching
more than one row on a declared "one" side — read-only queries,
diagnostics only, no constraints installed in the database.

## what was accomplished

### phase 1: spec + core check + seam method + backend (commit 2d2ec20)
- D05 specced in site/validation.md before implementation: direction
  table (many-to-one probes left, one-to-many probes right, one-to-one
  both independently with one problem each), zero matches never violate,
  NULL join columns pass, count = over-matched probe rows, and why
  equality joins are still measured despite the S06+D02/D03 overlap
- orientation normalization in core rich.rs: per-conjunct
  canonicalization (right-to-left conjuncts mirrored), then probe
  orientation (probing the right side mirrors ops again — Eq↔Eq, Ge↔Le,
  Gt↔Lt via `flip_op`); self-joins positional; under `--table` a
  relationship is in scope if it touches the selected table
- fifth `DuckdbBackend` seam method `count_overmatched_rows` — conjuncts
  cross the seam as data (`OrientedConjunct`: probe column, `JoinOp`,
  other column), never SQL text
- native impl in dbdict-duckdb: correlated count
  (`WHERE (SELECT count(*) FROM other o WHERE …) > 1`) with an
  empty-conjunct guard
- `ProblemKind::CardinalityViolation { count }` → D05, Level::Data,
  anchored at the join text + cardinality spans (the S06 two-span
  pattern); message names probe side, other side, and declared
  cardinality
- absent table/column skips mirror D04 (M06/M02 already reported);
  query failure → UnreadableSource at the join-text span
- tests red first: 274 → 290 (7 fake-backend, 9 real-duckdb incl. the
  overlapping-ranges motivating case, op-flip lock, NULL-pass,
  self-join, hostile-name quoting)

### phase 2: CLI e2e + docs (commit ba4e391)
- seeded rich-data CLI fixture gained a `periods` table with overlapping
  ranges and a many-to-one range relationship; snapshot shows D01–D05
  with D05 anchored at the relationship's cardinality declaration; the
  three trade dates cover over-match, exact-match, and zero-match
- clean fixture gained the same shape with non-overlapping periods plus
  a NULL join column — still exits 0, locking NULL-pass and
  zero-matches-pass semantics end to end
- README validate-data bullet and the spec.md `cardinality` bullet now
  reference D05 (the D03/D04 cross-reference pattern)

## key decisions

- all joins measured directly, not just range joins — the D02/D05
  double-report on duplicated equality-join pks accepted: the
  relationship-span diagnostic tells the user which declaration the
  data contradicts, and range joins get their only coverage
- severity error, consistent with D01–D04
- zero matches never violate — cardinality bounds multiplicity, not
  totality; unmatched fk rows are D04's business
- one-to-one checks both directions independently, one problem per
  violating direction, so the message can name which side over-matches
- the seam survives a fifth method but its shape stretched: D05 is the
  first check not describable as (table, column) pairs, so conjuncts
  cross the seam as data (columns + operator), never SQL strings —
  re-check at the sixth method

## insights captured

- .claude-work/insights/20260707-0953-composed-guarantees-and-orientation-normalization.md
- .claude-work/insights/20260707-1020-one-fixture-three-cardinality-outcomes.md
