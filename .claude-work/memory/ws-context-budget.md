---
name: ws-context-budget
description: "keep context under 30% during ws sessions; don't start a new phase at ≥25% — offer pause/restart instead"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: e63a384d-53c6-4384-b2fb-822c6b58fddb
---

During structured work sessions, keep context usage under 30%.

**Why:** the user prefers fresh sessions over degraded/compacted context for
phase transitions (see also global CLAUDE.md: "for major phase transitions,
prefer fresh session over /compact").

**How to apply:** check `/tmp/claude-context-pct.txt` at every phase
boundary (before starting the next phase, after `/ws done`). If ≥25%, do
NOT start the next phase — offer `/ws pause` + restart (`/state save` makes
this cheap). Related: [[rust-training-wheels-comments]].
