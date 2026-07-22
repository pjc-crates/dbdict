---
created: 2026-07-23T09:27:29+12:00
title: push output proves fast-forward vs force
tags: [git, gotcha, verification]
source: /ws done
---

## the push output is itself the proof of what happened

The push output is itself the proof of what happened. Git prints
`c1de1c8..881dae0` with a two-dot separator for a fast-forward, and
`+ c1de1c8...881dae0` with a leading `+` and three dots for a forced,
non-fast-forward update. Since the goal explicitly forbids force-pushing,
that separator is a free post-hoc assertion that no history was discarded —
worth reading rather than skimming on any push to a shared branch.

Used here on `git push origin duckdb-source:main`, which fast-forwarded
`main` from a single inherited upstream commit to the full 156-commit
project. The `..` confirmed git treated it as a true fast-forward, matching
the precondition checked beforehand with
`git merge-base --is-ancestor origin/main origin/duckdb-source`.

Belt and braces: the precondition check proves the FF is *possible* before
the push; the separator in the output proves it is *what actually happened*
after. Neither substitutes for the other — a `--force` would satisfy the
first check and still rewrite history.
