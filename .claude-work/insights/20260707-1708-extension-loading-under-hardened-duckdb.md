---
created: 2026-07-07T17:08:23+12:00
title: extension loading under hardened duckdb
tags: [rust, duckdb, gotcha, traits, workflow]
source: /ws done
---

## enable_external_access gates binaries, not LOAD
- duckdb's `enable_external_access(false)` gates loading external extension *binaries* (filesystem/network), not the `LOAD` statement itself — `LOAD json` succeeds when the extension is compiled in via the crate's `json` cargo feature
- so on a hardened build, "available extension" ≡ "statically linked", which makes M10's LOAD-attempt check exact: no `duckdb_extensions()` catalog query needed
- the autoload error we saw before the feature flag ("Cannot access directory ~/.duckdb/extensions/…") is the sandbox story the user predicted — autoinstall reaches for the filesystem/network and the hardening correctly refuses

## default trait methods as a seam-evolution tool
Adding `load_extensions` to `DuckdbBackend` with a default body ("everything
loads") meant the seven existing test fakes needed zero changes — only the
real backend overrides it, and the one test exercising M10 failure defines
its own overriding fake. A defaulted method is the low-churn way to grow a
widely-faked trait; the cost is that forgetting to override in the real
implementation compiles silently, so the default's doc comment must say
"the real backend must override".

## sed on repeated struct-literal patterns is a trap
A bulk `sed` keyed on `source: None,` at one indentation level hit both the
intended `DataDict { … }` literals and the same-shaped `Table { … }`
literals in the same files, inserting a field the second struct doesn't
have. Same-shaped lines at the same indentation need per-site edits or a
context-anchored (multi-line) pattern — and a `grep -c` sanity count after
any bulk edit.
