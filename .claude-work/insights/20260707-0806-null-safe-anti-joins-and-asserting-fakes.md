---
created: 2026-07-07T08:06:29+12:00
title: null-safe anti-joins and asserting fakes
tags: [rust, duckdb, testing, gotcha, traits]
source: /ws done
---

## trait methods without default bodies make fakes assert invariants

Note the deliberate absence of default method bodies on this trait: every
fake must explicitly implement each query method, and the meta-level/legacy
fakes use that to *assert* — `unreachable!("validate_meta must not run data
queries")` turns "this level never queries" from a convention into a tested
invariant. A default body would silently opt new fakes out of that
guarantee.

## NOT IN vs NOT EXISTS for anti-joins

With `NOT IN (SELECT pk ...)`, a single NULL in the pk column makes the
predicate NULL (neither true nor false) for every candidate row, and the
query silently reports zero orphans. `NOT EXISTS` is null-safe — it only
asks "is there a matching row", so NULL pk rows simply never match. D04's
native query uses `NOT EXISTS`, with a dedicated test
(`nulls_in_the_primary_key_column_do_not_mask_orphans`) locking the
behavior.
