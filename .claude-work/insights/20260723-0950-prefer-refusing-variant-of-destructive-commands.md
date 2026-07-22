---
created: 2026-07-23T09:50:33+12:00
title: prefer the refusing variant of destructive commands
tags: [git, gotcha, workflow, verification]
source: /ws done
---

## a guard chosen for one reason pays out for another

The plan specified `git branch -d` rather than `-D` as a generic "refuse if
unmerged" guard — not because any specific hazard was predicted. It then fired
on a situation the plan had not anticipated at all: a `/ws done` workflow
commits its own bookkeeping onto the branch it is about to delete, so `main`
could never be caught up at the moment the delete ran. `93eab52` ("Phase 2
done") existed on local `duckdb-source` and nowhere else.

`-D` would have silently discarded that commit and the session would have
looked clean. The refusal is what surfaced the gap, which was then fixed by
adding one fast-forward step before the deletes.

General form: prefer the refusing variant of any destructive command even when
confident it is unnecessary, because the cost of being wrong is asymmetric —
the refusing variant costs one extra round trip when you were right, and saves
the work when you were wrong.

## `git branch -d` prints the SHA on the way out

`Deleted branch duckdb-source (was 93eab52)` — printing the SHA is deliberate.
It is the last chance to recover a branch deleted by mistake: the ref is gone
but the commit stays in the reflog and remains reachable for the gc grace
period. Both the deletion message and the refusal message come from the same
merged-state check.

## workflows that commit their own bookkeeping change the merge math

Any phased workflow that writes a record commit at each phase boundary (here
`/ws done` → commit) makes the working branch permanently one commit ahead of
its integration target. A plan that ends with "delete the working branch" must
therefore include a final catch-up step *after* the last bookkeeping commit,
or the delete will refuse (with `-d`) or lose the record (with `-D`).

Worth checking at plan time for any session whose final phase removes the
branch the session is being recorded on.
