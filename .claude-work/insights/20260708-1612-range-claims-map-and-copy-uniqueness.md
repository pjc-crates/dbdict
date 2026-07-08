---
created: 2026-07-08T16:12:13+12:00
title: range claims map and copy uniqueness
tags: [rust, borrowing, gotcha, design, duckdb]
source: /state save
---

## claims map inverts role derivation; Vec find-or-insert borrowck limit

- The claims map `(table, column) → Role` inverts the data flow: roles were previously derived per-column from its own constraints, but range roles are imposed *from outside* by a relationship. Computing all claims up front, then consulting them first in the column loop, keeps the column loop single-pass and makes "column claimed twice" a natural map-insert conflict later.
- A Rust gotcha worth knowing here: the idiomatic "find-or-insert" on a `Vec` can't be written as `match vec.iter_mut().find(...) { Some(b) => b, None => { vec.push(...); ... } }` — the mutable borrow from `find` is considered live across the `None` arm, so borrowck rejects the push (a known NLL limitation the future Polonius borrow checker fixes). The plain workaround is finding the *position* first, then indexing.

## copies are only as distinct as their source; bounds are free

- There's a subtle correctness hole beyond the state dump's refusal list: a `SlotEqCopy` column *copies* the owner's value, so even on a one-to-one (injective) draw its values are only distinct if the **source** column's values are distinct. A unique-declared copy column with a plain-fill source would violate D03 — so the rule is "unique copy column ⇒ injective draw AND unique-implied source", not just "injective".
- Range *bound* columns need no unique check at all: their values are `nth(3i)`/`nth(3i+2)`, injective in the row index by construction — the same argument that makes `IndexedUnique` sound.
