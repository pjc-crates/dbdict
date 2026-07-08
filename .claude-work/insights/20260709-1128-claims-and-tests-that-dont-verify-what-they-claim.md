---
created: 2026-07-09T11:28:33+12:00
title: claims and tests that don't verify what they claim
tags: [testing, adversarial-review, gotcha, documentation, rust]
source: /ws done
---

## a substring assertion can pass on the fixture path, not the behavior

The pre-existing `ddl_refuses_a_legacy_dictionary` test asserted
`stderr.contains("legacy")` — and passed, but only because its temp dir was
named `dbdict-cli-ddl-legacy-<pid>` and that path is printed in the
diagnostic's `-->` line. It never reached the generate-level "cannot generate
DDL from a legacy dictionary" refusal it claimed to test. Writing the
equivalent `dummy` test with the *real* message assertion (`contains("rich")`)
exposed the gap immediately.

Lesson: a `contains()` assertion whose needle can appear in the fixture path,
temp-dir name, or the input echoed back is a false-positive trap. Assert on a
distinctive phrase from the actual message, and prefer one the fixture setup
cannot accidentally contain.

## a fixture can fail at an earlier layer than the one you mean to test

A `0.1.0` dict with a bare `type: number` fails **spec validation** (S07 wants
`examples`) inside `load_and_lower` — so it never reaches the generator's
`LegacyUnsupported` refusal at all. To exercise the generate-level path the
fixture had to be made *spec-valid* (add `examples:`). When a pipeline has
stacked refusal layers (spec → plan → generate), pick a fixture that is valid
at every layer above the one under test, or you're testing the wrong refusal.

## the exported "script that built the database" didn't actually build it

`dummy --sql` wrote only `Generated.script` (DDL + INSERTs), but `write_db`
also runs `LOAD <ext>` for each declared extension first — and `extensions` is
a *private* field never folded into `script`. So the README/comment claim
"the exact script that built the database" was false for any dict declaring an
extension. TDD missed it (my fixtures declared only json, which autoloads); the
adversarial review's cross-file tracer caught it by reading `write_db` and
comparing against the doc claim. Reinforces
[[20260709-0958-agent-review-catches-what-tdd-misses]]: a claim about a
deliverable is only as true as the code path that assembles it — verify the
artifact is self-contained, don't assert it in prose.

## atomic file replacement: write to a temp, then rename

`--force` originally did `remove_file(out)` then `write_db(out)` — a
delete-then-write window where a write failure leaves neither the old database
nor a new one. Fix: `write_db(tmp)` to a sibling temp on the same filesystem,
then `fs::rename(tmp, out)` (atomic on unix). This also sidesteps DuckDB's
stale `<out>.wal` sidecar, because a fresh temp has no prior WAL. The library
deliberately refuses to clobber an existing path; the CLI owning `--force` must
honor that intent *safely*, not by defeating the guard with an unguarded delete.

## rust 2024: `gen` is a reserved keyword

`let gen = ...` fails to compile under edition 2024 (`gen` is reserved for
generator blocks). rustc suggests the `r#gen` raw-identifier escape; just
rename. A quick trip-up when a natural variable name collides with a
newly-reserved word.
