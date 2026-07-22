---
created: 2026-07-23T10:09:24+12:00
title: self-recording workflows move their own target
tags: [workflow, git, gotcha, ws-close]
source: /state save
---

## exact commit counts are the wrong shape for a success criterion

The goal predicted `main` at 154 commits; it landed at 157. That gap is not
drift — it is the three bookkeeping commits the session wrote about itself
(`b951c73` open, `881dae0` phase 1, `93eab52` phase 2). A session that records
its own progress into the repo it is modifying moves its own target while
running.

Any success criterion phrased as an exact commit count is guaranteed to be
wrong by the number of phase boundaries. `>= 155` is the right form, and that
is what phase 2's verify actually used. Same root cause as the phase-3 plan
amendment (the delete refusing because `main` was one bookkeeping commit
behind) — worth checking at plan time for any session that both commits its
own record and asserts on repository shape.

## the memory sync is update-only, so it can silently under-report

`/state save`'s memory step runs `cp -ru CANONICAL/. LOCAL/` — recursive,
update-only, one-way, no delete. That means `MEMORY.md` is copied only when the
canonical copy is *newer*, which makes the index vulnerable in both directions:

- a canonical `MEMORY.md` listing fewer memories than the local one will
  overwrite it whenever canonical happens to be newer
- a local `MEMORY.md` that is newer silently wins, so a memory written this
  session never reaches the project index

Both states occurred here. The canonical directory did not exist before this
session, so it held 2 files and a 1-entry index while local held 6 files and a
4-entry index. The local index survived only by accident: `git checkout main`
during a 45-commit fast-forward rewrote `.claude-work/memory/MEMORY.md`, giving
it a newer mtime than the canonical copy written 14 hours earlier.

Repair is to make canonical genuinely complete — back-fill missing memory files
with `cp -n` (never overwrite), rewrite `MEMORY.md` to index everything, then
re-run the normal canonical→local copy. Worth checking `ls` counts and
`grep -c '^- \['` on both sides after any `/state save`, rather than trusting
the reported file count, which counts files and not index entries.
