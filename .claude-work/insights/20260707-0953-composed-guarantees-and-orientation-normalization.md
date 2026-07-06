---
created: 2026-07-07T09:53:12+12:00
title: composed guarantees and orientation normalization
tags: [design, testing, duckdb, rust]
source: /ws done
---

## reading adjacent checks first changed the session's shape

- Reading S06 first changed the shape of this session: S06 already
  *requires* the "one" side of any declared cardinality to carry
  `primary_key` or `unique` — and D02/D03 already verify those
  constraints against the data. So for **equality joins**, cardinality
  verification composes for free: S06 + D02/D03 passing implies the data
  honors the declared cardinality.
- The genuine gap is **range/multi-conjunct joins**: S06 is deliberately
  permissive there (`validate_spec.rs` — "any column on the one side
  unique-implied"), and a unique `period.start` does nothing to prevent
  *overlapping ranges*, which silently violate `many-to-one` in the
  data. No existing check catches that.
- This also means D05's query is a different beast from D01–D04: it must
  evaluate the join expression as declared (including range conjuncts),
  not just count duplicates in one column — which pressures the
  narrow-seam design in a new way.

## double-flip orientation, correlated counts, NULLs for free

- The double-flip in `check_relationships_data` is the subtle bit:
  conjuncts first get *canonicalized* (lhs on the join's left table — a
  conjunct written `b.y <= a.x` becomes `a.x >= b.y`), then *oriented*
  for the probe (probing the right side mirrors again). Self-joins fall
  out for free because both sides name the same table, making every
  conjunct trivially canonical — orientation is positional.
- The correlated-count query (`WHERE (SELECT count(*) FROM other o
  WHERE …) > 1`) reads as the spec sentence: "rows matching more than
  one row". A `JOIN … GROUP BY … HAVING` version would need a synthetic
  row key (`rowid`) to survive the aggregation; the correlated shape
  sidesteps that entirely.
- NULL handling needed zero code anywhere: a NULL join column fails
  every comparison, so the subquery counts 0 matches, and 0 < 2. The
  semantics decision ("zero matches OK") and SQL's comparison semantics
  compose so the right behavior is the default — locked by
  `null_join_columns_match_nothing` rather than implemented.
