---
created: 2026-07-07T07:09:44+12:00
title: explicit drop flushes duckdb wal before spawned binary reads
tags: [rust, duckdb, testing, gotcha]
source: /ws done
---

## explicit drop(conn) is load-bearing in e2e fixtures

These e2e tests build the fixture DB with the *bundled* `duckdb` crate
in-process, then hand the file to the spawned binary — note the explicit
`drop(conn)` before spawning. Rust would drop the connection at end of
scope anyway, but the binary opens the file *mid-function*, so the early
drop is load-bearing: it forces DuckDB to flush its WAL before the
read-only open.
