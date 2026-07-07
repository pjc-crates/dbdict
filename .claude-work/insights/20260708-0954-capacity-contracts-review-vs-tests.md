---
created: 2026-07-08T09:54:30+12:00
title: capacity contracts, layer-honest refusals, review vs tests
tags: [rust, duckdb, testing, gotcha, adversarial-review]
source: /ws done
---

## pattern-lumping capacity bugs read as correct

- Why the split mattered: `BigInt | HugeInt => u64::MAX` was a
  pattern-match lumping error, not a math error — HUGEINT (int128)
  genuinely holds every u64 index, so the *pair* was half right, which is
  exactly the kind of bug that reads as correct in review-by-eye.
- The `Union` fix moves capacity from "min of all parts" (right for
  STRUCT, where one index drives *every* field) to "first alternative
  only" (right for UNION, where `nth` picks one tag) — capacity must
  mirror what `nth` actually does, not the type's theoretical width.

## invariants must be honest about their layer; tests pin empirical bounds

- The capacity check couldn't go in `plan()` without breaking the
  architecture — that crate never parses type strings by design. When a
  documented invariant ("rendering cannot fail") is unenforceable at its
  stated layer, the fix is *either* machinery to enforce it there or an
  honest re-statement; we chose the backend check + doc fix, which keeps
  the crates decoupled.
- Note what the RED runs bought: the interval test would have caught it if
  the verifier's empirical claim (2^31 literal bound) had been wrong — the
  test asserts the engine accepts `nth(cap - 1)`, so the constant is
  pinned by the engine, not by the review.

## green oracle tests are not range coverage

The multi-agent review found five confirmed bugs that a green oracle-test
suite missed, because the tests only sampled small indices (0..20) while
plain-fill draws span the full u64 range — "tests pass" and "contracts
hold across the input range" are different claims. When a value is drawn
as `hash % capacity`, the top of the range is the common case, not an
edge case: test at `capacity - 1`, not just at 0..n.
