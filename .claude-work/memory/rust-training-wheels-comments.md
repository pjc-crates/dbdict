---
name: rust-training-wheels-comments
description: "This user is learning Rust — wants thorough \"training-wheels\" comments, plain (not fancy) Rust, and maintenance over execution speed"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: 6b1e9459-ab57-45b6-a5df-46affe2c0ed5
---

The user is new to Rust and learning as they go. On Rust code for their projects, write "training-wheels" comments and keep the code plain.

**Why:** they're learning the language and optimize for maintenance, bug-chasing, and feature-addition cost over execution speed.

**How to apply:**
- comment thoroughly but concisely — complete, not padded
- lowercase comments; no trailing period on end-of-line comments
- no fancy/clever Rust — keep it explicit and clear (avoid dense iterator chains, macro tricks, lifetime gymnastics where a plain version reads better)
- explain the *why*, and call out Rust idioms/gotchas inline, since they're learning
- readability and low maintenance cost beat raw performance

First arose in the dbdict project (a duckdb rich-type data dictionary — the fork that diverged from tidyverse data-dict). Consistent with the user's global CLAUDE.md ("Rust, Go: early learner — explain idioms, gotchas, and why").
