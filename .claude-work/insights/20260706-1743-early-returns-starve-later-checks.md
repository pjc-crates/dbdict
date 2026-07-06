---
created: 2026-07-06T17:43:25+12:00
title: early returns starve later checks
tags: [rust, gotcha, pattern]
source: /ws done
---

## early returns in a shared check function silently starve later checks

`check_table_data`'s D02 block used `return` for its guard clauses (no
primary key; key column missing from the db). Appending D03 after it would
have meant D03 never ran for pk-less tables — the exact tables most likely
to rely on `unique`. The fix was restructuring D02's guards into scoped
`if` blocks before adding D03, with a comment saying why.

General shape: when a function is a *sequence of independent checks*, guard
clauses must be per-check scopes, not function-level returns — the next
check added pays for the shortcut.
