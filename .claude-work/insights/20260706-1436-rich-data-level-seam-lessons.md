---
created: 2026-07-06T14:36:26+12:00
title: rich data level seam lessons
tags: [rust, duckdb, design, traits]
source: /ws done
---

## quiet re-resolution over threaded state
- The rich data level re-resolves the source **quietly** instead of threading state out of `check_meta`: M04/M05/M06 were already reported there, so a missing piece in `check_data` just means "nothing to query." Slight re-read of the schema, zero API contortion — readability over speed, per the project's conventions.

## query with the database's spelling, report at the dictionary's spans
- Queries use the **database's** spelling of names while problems point at the **dictionary's** spans — the two sides of `names_eq` each serve the audience that owns them (the db executes, the human reads their own YAML).
- Proven against real duckdb: quoted identifiers still match case-insensitively (`"trades"` finds `Trades`), so exact db-side spellings keep `quote_ident` trivially safe — including for hostile names like `we"ird; DROP TABLE x`.
