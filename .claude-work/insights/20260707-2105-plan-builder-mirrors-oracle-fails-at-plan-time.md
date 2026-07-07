---
created: 2026-07-07T21:05:52+12:00
title: plan builder mirrors oracle, fails at plan time
tags: [rust, duckdb, design, pattern, error-handling]
source: /ws done
---

## cardinality analysis mirrors the D05 oracle positionally, so plan and validator cannot drift

- The cardinality analysis mirrors `rich.rs` D05 *positionally*, not by table name: `one-to-one` self-joins name the same table on both sides, so "which side" must be tracked as left/right booleans (like `probe_left_directions` in `rich.rs:419`) or a self-join would check the same column list twice.
- Because roles derive injectivity from declared constraints (`unique`/`primary_key` → `IndexedUnique` or injective `FkDraw`), "verify the one side is unique" is a pure constraint check — no data reasoning needed. The oracle and the plan can't drift as long as both key off `is_unique_implied()`.

## fail-at-plan-time boundary keeps the renderer infallible

- The plan is deliberately **fail-at-plan-time**: every constraint that could make generation impossible (pigeonhole on injective draws, empty targets, cycles) is checked before a `Plan` exists, so phase 4's renderer can be infallible with respect to constraints — a common Rust pattern of pushing errors to a validation boundary so downstream code needs no `Result`s.
- `FkDraw` doesn't carry `target_rows` — phase 4 looks it up in the same `Plan`. Duplicating it would create two sources of truth that could disagree after a future edit; a lookup can't drift.
