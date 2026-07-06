# D05 cardinality verification

## problem

`cardinality` is declared on every relationship but never verified against
the data. S06 checks it is *consistent with declared constraints* (the
"one" side must be `primary_key` or `unique`), and D02/D03 verify those
constraints hold in the data — so for **equality joins** the guarantees
compose. But for **range/multi-conjunct joins** S06 is deliberately
permissive (any one-side conjunct column unique-implied suffices), and
nothing prevents *overlapping ranges*: a `many-to-one` date-range join
where one event date falls inside two periods silently violates the
declared cardinality. No existing check runs the join and counts.

Like D01–D04: read-only queries, diagnostics only — **no constraints
installed in the database**.

## success criteria

- **D05** (error, rich format only): the data violates a relationship's
  declared `cardinality` — some row matches more than one row on a
  declared "one" side when the join is actually evaluated.
  - checked by *direct measurement* for **all** join types (equality and
    range/multi-conjunct): evaluate the relationship's join expression as
    declared and count matches per row.

> decision (user, 2026-07-07): all joins measured directly, not just
> range joins. For an equality join this is redundant with S06+D02/D03,
> and a duplicated pk may double-report (D02 at the column *and* D05 at
> the relationship) — accepted: the relationship-span diagnostic tells
> the user *which declaration* the data contradicts, and range joins get
> their only coverage.

> decision (user, 2026-07-07): severity is **error**, consistent with
> D01–D04 — a declaration the data contradicts fails the run.

  - direction semantics (cardinality reads left→right in the join text):
    - `many-to-one`: each left row matches at most one right row
    - `one-to-many`: each right row matches at most one left row
    - `one-to-one`: both directions, checked independently — one D05
      problem per violating direction

> decision (user, 2026-07-07): `one-to-one` checks both directions,
> mirroring S06's both-sides rule; each direction reports its own
> problem so the message can name which side over-matches.

  - **zero matches are not a violation** — cardinality bounds
    multiplicity (≤1), not totality. Unmatched fk rows are D04's
    business; other relationships may legitimately have unmatched rows.

> decision (user, 2026-07-07): zero matches OK. Mirrors SQL: a join
> without an fk carries no totality guarantee.

  - NULLs need no special handling: a NULL join column never satisfies
    an equality or range conjunct (SQL comparison semantics), so such
    rows match zero rows and pass under the zero-matches rule.
  - count reported = number of rows (on the "many"/probing side) that
    match more than one row — proposed; mirrors the D02–D04 "count what
    violates" convention at row granularity, since D05 is about rows
    over-matching rather than duplicate values.
- specced in site/validation.md *before* implementation (D02–D04
  convention), including the direction table, the zero-match rule, and
  the relationship to S06/D02/D03 (why equality joins still get checked).
- `dbdict validate-data` reports D05 anchored at the relationship
  (join text + cardinality spans, as S06 anchors); a seeded fixture with
  overlapping ranges fails; the clean fixture still passes.
- `cargo test --workspace` green; clippy + rustfmt clean.

## scope

- in:
  - D05 spec text in site/validation.md
  - rich data level: a fifth `DuckdbBackend` seam method that evaluates a
    join and counts over-matching rows — exact signature is a planning
    question: unlike D01–D04's (table, column) shape, D05 must carry the
    full conjunct list (equality and range operators) across the seam
    without leaking SQL into core
  - relationship resolution in core `check_data`: skip a relationship if
    either table is absent from the db (M06 covers) or any join column is
    absent (M02 covers) — the D04 skip pattern
  - fake-backend tests in core, real-duckdb tests in dbdict-duckdb
    (overlapping ranges, equality duplicates, self-join, NULL join
    columns, hostile-name quoting, case-insensitive identifiers),
    CLI e2e snapshot update
  - README validate-data sentence mentions D05 alongside D01–D04
- out:
  - totality / exactly-one verification (zero matches never violate)
  - `conflicts` verification against data — unrelated to cardinality
  - legacy (0.1.0) path — preserved, not extended (same call as D02–D04)
  - suppressing the D02/D05 double-report on duplicated equality-join
    pks — accepted as designed (see decision above)

## constraints

- **no constraints installed in the database** — read-only SQL only;
  the types-only DDL decision stands
- core `dbdict` stays duckdb-free; the join-evaluation query lives
  behind the backend trait — the seam signature must express conjuncts
  as data (columns + operator), not SQL strings
- rich queries use db-side name spellings on both tables; problems
  locate at dict spans (relationship join/cardinality spans)
- legacy behaviour unchanged (existing tests keep passing)
- TDD, red first; maintainer is learning Rust — training-wheels comments
