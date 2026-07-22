---
name: git-two-identities-machine-tracking
description: "the author/committer identity split is deliberate machine tracking — never flag it as misconfiguration or offer to \"fix\" it"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 9f733188-d4b8-460c-a61a-a8a3bafd1c78
  modified: 2026-07-22T07:27:03.870Z
---

Git commits here carry two different identities on purpose:

- **author** — `Peter Crosbie <pjcrosbie@gmail.com>` (the real identity, what
  reads cleanly in `git log`)
- **committer** — `pjc on <hostname>`, e.g. `pjc on thelio25
  <pjc@thelio25.local>` (which machine the commit was made on)

`git tag -a` takes its **tagger** field from the *committer* identity, so
annotated tags show `pjc on thelio25` rather than `Peter Crosbie`. That is
correct, not drift.

**Why:** the user tracks which machine a commit was made on without cluttering
the commit message with that information. Stated verbatim 2026-07-22: "there
are two identities in git, I use different name so I can track which machine a
commit was done on without cluttering up the commit message."

**How to apply:** never report the mismatch as a problem, never offer to retag
or amend to "align" the identities, and never set `user.name`/`user.email` to
make them match. Note that `git config user.name` shows only the author
identity — seeing `Peter Crosbie` there while a tag says `pjc on thelio25` is
the expected state, not a contradiction worth investigating. Related:
[[rust-training-wheels-comments]].
