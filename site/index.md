---
title: "dbdict.yaml"
---

`dbdict.yaml` is a DuckDB-native data dictionary: a YAML document that
describes a collection of related tables — their columns, exact types,
constraints, relationships, and the specialised vocabulary you need to
understand them. It is designed to be a living document, co-written by humans
and agents, that tracks your understanding of a dataset as it evolves.

Types are first-class. A `typedef:` layer lets you name domain types
(`money: DECIMAL(18, 4)`) and compose them into structs, enums, arrays, and
maps — and validation round-trips the dictionary through DuckDB itself, so the
types in the dictionary match the database exactly, not approximately. The
`dbdict` CLI validates a dictionary at three levels: against the spec, against
the database's schema, and against the data's actual values (nulls in
`required` columns, duplicated keys, orphaned foreign keys, violated
cardinality). It can also generate an executable DDL script from the
dictionary, and generate a DuckDB database of dummy data whose values satisfy
every declared constraint by construction. See [the specification](spec.md)
and [validation](validation.md)
for details, or the [CLI](https://github.com/pjc-crates/dbdict#readme) to get
started.

## Lineage

`dbdict` is a fork of tidyverse
[data-dict](https://github.com/tidyverse/data-dict) (MIT) that deliberately
diverges from it: where `data-dict.yaml` aims to stay lightweight and
backend-neutral, `dbdict.yaml` commits to DuckDB and trades portability for
exact type fidelity. The legacy `data-dict.yaml` format (v0.1.0, coarse
semantic types, parquet sources) is still parsed and validated, so existing
dictionaries keep working — see these legacy-format examples from upstream:

* [dabstep](examples/dabstep.qmd)
* [elevators](examples/elevators.qmd)
* [foodbank](examples/foodbank.qmd)
* [loan-application](examples/loan-application.qmd)
* [otters](examples/otters.qmd)

## Why a data dictionary?

There have been many previous attempts to encode data dictionaries in
structured text. What makes this approach different? Why revisit this problem
now?

* The costs of creating a data dictionary are lower than ever before because
  AI agents can automate much of the boilerplate, including porting
  documentation from existing unstructured formats (e.g. `.doc`, `.html`,
  `.pdf`).
* The benefits of creating a data dictionary are higher, because AI agents
  need the context that currently exists only in your head. As a very pleasant
  side-effect, this also helps your human colleagues, particularly those who
  are newer to your organisation.
* LLMs change what it means for something to be machine readable. While we
  explicitly encode the most important structures, we can leave the more
  unusual quirks to free-form text.
* Committing to a single database engine means many parsing and portability
  details are out of scope: the dictionary speaks DuckDB's own type language,
  and the database itself is the authority on what a type means.
* The cost of describing the data semantics in multiple places (i.e.
  `dbdict.yaml` and data transformation code) is lower because an AI agent can
  easily keep both in sync.

## Inspirations

Here are a few of the resources that guided the design of the format:

* [Data management in large-scale education research](https://datamgmtinedresearch.com/document#document-dataset)
* [Frictionless data](https://datapackage.org/standard/table-schema)
* [Hex's semantic modelling](https://learn.hex.tech/docs/connect-to-data/semantic-models/semantic-authoring/modeling-specification)
* [Snowflake's semantic views](https://docs.snowflake.com/en/user-guide/views-semantic/overview)
* [Soda's contract language](https://docs.soda.io/reference/contract-language-reference)
* [dbt tests](https://docs.getdbt.com/docs/build/data-tests?version=1.12)

It's worth noting that while semantic models influenced the design of the
format, it is not a **[semantic model](semantic-models.md)**. This means it
doesn't think about dimensions or metrics, because that distinction reflects
intended use, not the data itself. It's primarily designed to support data
scientists, not data analysts.

Additionally, and while terminology is still evolving, the "semantic" in
semantic models is typically interpreted narrowly, focussing on structural
semantics (what's needed for queries to return consistent values) not what the
data actually _means_.

## Direction

The validator, the DDL generator, and the dummy-data generator ship today (see
the [CLI](https://github.com/pjc-crates/dbdict#readme)) — the last generating
sample data that satisfies the declared types, constraints, and relationship
cardinalities by construction. The architecture separates the dictionary model
from the tools that consume it, and more consumers are planned:

* **Code generation**: emit Python and Julia bindings for a dictionary's
  tables and types.
* **User facing documentation**: turn a maintained dictionary into attractive
  HTML documentation of your data.
