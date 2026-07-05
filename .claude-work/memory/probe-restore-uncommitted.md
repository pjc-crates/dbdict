---
name: probe-restore-uncommitted
description: never git-checkout to undo a probe edit while the work is uncommitted — snapshot the file first
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 81d973a8-4a61-4198-a77c-7eb9acbcfa01
---

While empirically testing a code-review claim (temporarily neutering a code
path), I restored the probed file with `git checkout -- <file>` — which
reverts to HEAD and wiped the session's uncommitted phase-2 changes to that
file. Recovery required reconstructing every edit from context.

**Why:** `git checkout --` restores HEAD state, not "the state before my last
edit". During a work session the working tree is routinely ahead of HEAD, so
checkout is destructive there.

**How to apply:** before any temporary probe edit, `cp` the file to the
scratchpad (or `git stash push -- <file>` / apply the probe via a patch that
can be reversed). Restore from that snapshot, never from HEAD. Related:
[[rust-training-wheels-comments]] project uses phase-end commits — probes are
safest right after a `/ws done` commit.
