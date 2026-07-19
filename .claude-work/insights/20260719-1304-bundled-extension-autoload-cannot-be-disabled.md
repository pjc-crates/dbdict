---
created: 2026-07-19T13:04:43+12:00
title: a statically-linked duckdb extension can't be gated by autoload settings
tags: [duckdb, testing, rust, gotcha, adversarial-review]
source: /code-review (session close)
---

## disabling autoload does not force an explicit LOAD for a bundled extension

The `duckdb` crate here is built with `features = ["bundled", "json"]`, so json is
**statically linked into the binary**. Its `JSON` type and functions are always
available — `SET autoload_known_extensions=false; SET autoinstall_known_extensions=false;`
does **not** remove them. Those settings gate the *dynamic* autoload/autoinstall of
*installable* extensions; a compiled-in extension is unaffected.

I hit this trying to strengthen the `--sql` self-containment round-trip test. The
plausible-sounding fix — "disable autoload so the script's explicit `LOAD json;`
becomes required" — was **refuted by a 30-second probe**: with the `LOAD` fold
removed *and* autoload disabled, the round-trip still passed (the JSON column's
`CREATE` resolved via the statically-linked type). So no connection setting can
make json's `LOAD` observably necessary in this build.

Consequence for tests: when the only offline-loadable extension is a *bundled*
one, a round-trip cannot prove an explicit `LOAD` is emitted — a **text/AST
assertion on the generated script** (`contains("LOAD json;")`) is the only
autoload-independent guard. The round-trip still earns its place as an
*executability* check (valid DDL/INSERTs, well-formed `LOAD`), just not as the
regression guard for the fold. Direct application of
[[20260709-1128-claims-and-tests-that-dont-verify-what-they-claim]] — this time to
my own "fix," caught before it shipped.

Meta-lesson (reinforces [[agent-review-catches-what-tdd-misses]] in spirit): when a
test change rests on an assumption about external-engine behavior, *probe it* —
neutralize the guard, remove the feature, and watch the test. Don't reason it green.
