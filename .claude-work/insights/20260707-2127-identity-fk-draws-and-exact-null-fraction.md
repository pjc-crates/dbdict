---
created: 2026-07-07T21:27:33+12:00
title: identity fk draws and exact null-fraction edges
tags: [rust, duckdb, design, gotcha]
source: /ws done
---

## identity convention for injective fk draws keeps value resolution database-free

- The identity convention for injective FK draws (`k = i`) is what makes value resolution database-free: every FK target is a PK column, so `stored_value` follows a chain of identity draws down to an index-generated column and just computes `nth(ty, i)`. A permuted injective draw would have forced reading data back or materializing row values.

## decide null_fraction >= 1.0 exactly, not through the float comparison

- `null_fraction >= 1.0` is decided exactly rather than through the float comparison — `(hash as f64) < 1.0 * u64::MAX as f64` can miss top-end hash values when the cast rounds up to 2⁶⁴, which would make "all NULLs" flaky in a way that's brutal to debug later.
