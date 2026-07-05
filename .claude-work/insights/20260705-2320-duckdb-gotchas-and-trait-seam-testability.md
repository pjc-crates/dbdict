---
created: 2026-07-05T23:20:00+12:00
title: duckdb execute/case gotchas and the trait-seam testability pattern
tags: [duckdb, security, gotcha, rust, traits, testing, adversarial-review]
source: /ws done
---

## duckdb-rs `Connection::execute` runs EVERY statement in the string, not just the first

Verified against the bundled `duckdb` crate (v1.10504.0 = DuckDB 1.5.4):
`Connection::execute` → `InnerConnection::prepare`, whose own source comment
reads "Extract statements (handles both single and multi-statement queries)"
and "Execute all intermediate statements". So `execute("CREATE TYPE x AS INTEGER;
ATTACH '/path' AS c; CREATE TABLE c.t(...)")` returns `Ok` and runs **all three**.
There is an `Error::MultipleStatement` variant but it is never raised on this path.

Consequence for dbdict: the rich round-trip interpolates the user's own `typedef:`
/ column `type:` text into `CREATE TYPE`/`CREATE TABLE` DDL. A malicious dictionary
(a data dict is a *shared* artifact — untrusted) could smuggle `ATTACH`/`COPY` and
reach the filesystem. Empirically confirmed: an `ATTACH` in a column type created
a file on disk.

I had written a code comment claiming `execute` "runs a single statement, so a
smuggled second statement is rejected" — flatly false. Three reviewers + my own
firsthand check caught it. Lesson reinforced (CLAUDE.md sourcing rule): never
state external-tool behavior from memory in a comment; the false claim would have
been *relied on* later.

**The fix:** open scratch connections with
`Config::default().enable_external_access(false)` via
`Connection::open_in_memory_with_flags`. That blocks ATTACH/COPY/read_csv while
normal type expressions still work. Combined with the scratch db being throwaway
in-memory and the real db opened read-only, a hostile dict can at worst make
instantiation fail. Also guard the probe: require the per-column DESCRIBE to
return exactly one row (a top-level comma in a type smuggles phantom columns).

## duckdb identifiers are case-insensitive but case-preserving in DESCRIBE

`CREATE TABLE Trades (Qty INTEGER)` then `DESCRIBE Trades` / `information_schema.
tables` return the **stored** spelling (`Trades`, `Qty`), but `SELECT ... FROM
trades` resolves fine — lookups fold case. So comparing DESCRIBE output against a
dictionary's names with exact Rust `==` is a bug: a lowercase `dbdict.yaml` vs a
`CamelCase` database produces spurious "missing"/"undocumented" diffs (here
M02/M03/M06/M07).

Fix: fold case (ASCII `eq_ignore_ascii_case` covers ascii identifiers — the
practical universe) on every dict↔db name match, but NOT on same-source
comparisons (the scratch "expected" side carries the dict's own spelling) and NOT
on type strings (canonical types are already normalised — `BIGINT`, `DECIMAL(12,2)`).

## trait-seam keeps the core crate pure AND makes the whole diff TDD-able

Pattern that worked well: the core crate (`dbdict`) defines a `DuckdbBackend`
trait (`instantiate` / `read_schema` / `classify`) plus plain data types
(`Instantiated`, `TableSchema`, `TypeCategory`), and the heavy crate
(`dbdict-duckdb`, which links the bundled C++ duckdb) implements it. The CLI wires
the real impl in.

Payoff: the *entire* diff logic (M01–M09, the descriptive-key checks) was built
test-first against a `FakeDuckdb` returning canned schemas — no multi-minute
duckdb build in the core test loop, and every problem-at-a-span pinned precisely.
The real backend is proven separately by end-to-end tests. The classifier lives on
the trait (duckdb owns canonical spellings), so core stays free of type-spelling
knowledge; the core tests' `fixture_classify` is not a drift risk because it only
feeds the diff logic — the real classifier is pinned against live DESCRIBE in the
duckdb crate.

Gotcha the review surfaced: a seam type `Vec<Option<T>>` where no code path ever
produces `None` is dead generality — it forces `.and_then(Option::as_ref)` at every
call site and an untested "None" arm. If nothing fills the `None`, drop the
`Option`. "Future-proofing" that no producer honours is a readability tax now for a
maybe later.
