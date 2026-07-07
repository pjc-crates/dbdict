---
created: 2026-07-07T11:59:17+12:00
title: snapshot URL exposure and fixpoint acceptance
tags: [rust, testing, workflow]
source: /ws done
---

## snapshot context windows decide which fixtures leak into snapshots
- Only ~20 of the 36 URL-bearing snapshots actually changed: a diagnostic snapshot embeds the fixture's `$learn_more` line only when the rendered context window (lines around the problem's span) happens to reach line 2 of the YAML. The rest contain the URL merely because their *fixture path* is echoed — the sed swept those fixtures but their rendered excerpts never showed the URL line.

## accepting snapshots without cargo-insta is a fixpoint loop
- Without `cargo-insta`, plain `cargo test` surfaces pending snapshots a few at a time (insta stops evaluating later snapshot assertions in a failing process), so acceptance is a fixpoint loop: accept-reviewed → re-run → repeat until green. Automating the review with a diff whitelist (`tidyverse|pjc-wspace/dbdict|assertion_line`) keeps the loop honest.
