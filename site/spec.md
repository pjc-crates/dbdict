# dbdict.yaml

This document specifies the `dbdict.yaml` data dictionary format. There are two
versions of the spec, selected by the required `$version` key:

* **`0.2.0` (rich)** — the current format, described first in this document.
  Columns are typed in DuckDB's own type system — `STRUCT`, `ENUM`, `LIST`,
  arrays, `DECIMAL(p,s)`, and so on — with a `typedef:` alias layer for naming
  and reusing types. One dictionary describes one DuckDB database, and
  validation compares declared types against the database with full fidelity.
* **`0.1.0` (legacy)** — the original `data-dict.yaml` format: coarse semantic
  types (`number`, `string`, …) validated against per-table Parquet files. It
  is preserved so existing files keep validating; see
  [The legacy format](#the-legacy-format-010) for where it differs.

The CLI looks for a `dbdict.yaml` in the target directory, falling back to the
legacy file name `data-dict.yaml`. The *format* is chosen by `$version` alone,
not by the file name.

A data dictionary has three kinds of top-level keys. `$`-prefixed metadata keys
that describe the dictionary itself, descriptive keys that name and describe
the dataset as a whole, and content keys that describe the data. The `$` prefix
marks a key as meta, distinguishes it from content, and keeps these keys
grouped at the top of the file.

The metadata keys are:

* `$version` (required): the version of the spec the document conforms to —
  `0.2.0` for the rich format, `0.1.0` for the legacy format. While the spec is
  pre-1.0, breaking changes are expected, but once the spec stabilises at 1.0,
  breaking changes will always increment at least the minor version.
* `$learn_more` (optional, but recommended): a URL where readers can learn
  about the format, so that people and tools meeting the file for the first
  time can find out what it is. Use <https://github.com/pjc-crates/dbdict>. Omitting
  it is valid, but a validator will emit a warning rather than an error (see
  [Validation](validation.md)).

The descriptive keys identify and document the dataset as a whole:

* `name` (optional): a human-readable name for the dataset, suitable for
  display in a user interface that lists several dictionaries. Unlike a table
  name, it has no uniqueness or character constraints — it's a title, not an
  identifier.
* `description` (optional): a short, human-readable description of the dataset.
  May contain markdown, and is usually a few sentences or a paragraph.
* `details` (optional): additional information about the dataset. Can be any
  length.

In the common case of a dictionary that describes a single table, these
top-level keys should be used to describe the dataset, leaving the table itself
undescribed.

The content keys all hold the actual information about the data:

* [`typedef`](#typedefs) names reusable DuckDB type expressions that column
  `type:`s (and other typedefs) can refer to.
* [`source`](#source) names the DuckDB database the dictionary describes.
* [`duckdb`](#duckdb-extensions) declares the DuckDB extensions the
  dictionary's types depend on.
* [`tables`](#tables) is where the bulk of most dictionaries will be. It
  describes the tables and their columns.
* [`relationships`](#relationships) describes the relationships between tables.
  It gives the details you need to safely create joins.
* [`glossary`](#glossary) provides a place to define important domain-specific
  terms. This is a good place to write down those special words that your
  company loves to use.
* [`version`](#version) records the version of the data the dictionary
  describes — a version number, a date, or an opaque hash.

`name`, `description`, and `details` form a consistent trio that recurs at
every level of the dictionary: the dataset as a whole (here), each
[table](#tables), and each [column](#columns). `description` and `details` are
always optional and mean the same thing at every level — a short summary and a
longer free-text note.

## Typedefs

`typedef` is a map from alias name to a native DuckDB type expression. An alias
gives a domain name to a type you use repeatedly, so the dictionary reads in
the domain's vocabulary and a change to the underlying type is made in one
place:

```yaml
typedef:
  money: DECIMAL(18, 4)
  address: STRUCT(city VARCHAR, postcode INTEGER)
  price_history: money[]
```

* The expression is DuckDB SQL, exactly as you would write it in a
  `CREATE TABLE` — copy-pasteable in both directions.
* Aliases **compound**: an alias may appear inside another alias's expression
  (`price_history` above). Declaration order does not matter.
* A table may declare its own `typedef` map (see [Tables](#tables)); a
  table-scoped alias **shadows** a global alias of the same name for that
  table's columns.

Alias expansion is delegated to DuckDB itself: validation instantiates each
typedef with [`CREATE TYPE`](https://duckdb.org/docs/current/sql/statements/create_type.html)
in a scratch in-memory database, so exactly what DuckDB accepts is what the
dictionary accepts. A typedef DuckDB rejects — an unknown or cyclic reference,
or a malformed expression — is reported with DuckDB's own error at the
typedef's definition (see M08 in [Validation](validation.md)). Run
`dbdict resolve` to print every alias's canonical expansion.

The dictionary is executable in the other direction too: `dbdict ddl`
generates a flat DuckDB script — `CREATE TYPE` per typedef in dependency
order, then `CREATE TABLE` per table — proven runnable against a scratch
in-memory database before it is printed. Because `CREATE TYPE` names are
database-global, a table-scoped alias that shadows another alias's name
cannot be spelled in one flat script; `ddl` refuses with an error naming the
colliding typedefs rather than renaming them for you.

The generated tables carry types only: column constraints (`primary_key`,
`required`, `unique`) are deliberately not emitted as SQL clauses. A schema
created from a dictionary exists mostly to be bulk-loaded, and the
[DuckDB performance guide](https://duckdb.org/docs/current/guides/performance/schema.html)
advises "For best bulk load performance, avoid primary key constraints".
dbdict's model is declare-then-check: constraints stay in the dictionary as
declarations, and `dbdict validate-data` verifies the loaded data by query
(see [Validation](validation.md)) instead of the database enforcing them row
by row during the load.

## Source

`source` names the data the dictionary describes: one dictionary describes one
DuckDB database.

```yaml
source:
  duckdb:
    file: warehouse.duckdb
```

* `duckdb.file`: path to a DuckDB database file. A relative path is resolved
  relative to the dictionary file; an absolute path is used as-is.

Each table in the dictionary is matched to the database relation with the same
name — either a table or a view. Names are matched case-insensitively (ASCII
folding), as DuckDB identifiers are. The database is always opened read-only:
validation never creates, mutates, or locks it for writing.

`source` is optional while you're only validating the spec, letting you sketch
a dictionary before its database exists. But the metadata level validates the
dictionary against the real database, so it requires a `source` naming a
readable database.

## DuckDB extensions

`duckdb.extensions` declares the
[DuckDB extensions](https://duckdb.org/docs/current/core_extensions/overview.html)
the dictionary's types depend on — for example `json` for JSON columns:

```yaml
duckdb:
  extensions:
    - json
```

Declared extensions are `LOAD`ed into every engine connection dbdict opens
for the dictionary, and the metadata level checks that each one actually
loads on the local engine, reporting M10 with DuckDB's own error when one
does not (see [Validation](validation.md)).

* A name must be lowercase ASCII letters, digits, or underscores (S19) —
  the conservative shape that covers real extension names and keeps the
  name safe to place in a `LOAD` statement. Declaring the same extension
  twice is a warning (S20).
* Declaring is LOAD-only: dbdict never `INSTALL`s an extension, so it never
  fetches one from the network. Validation runs with DuckDB's external
  access disabled (a dictionary is untrusted input), which also blocks
  loading extension binaries from disk — an extension is available to
  validation only when it is compiled into dbdict's bundled engine. `json`
  is compiled in.

## Tables

`tables` is a list that describes each table in the dataset. Each table
represents a rectangle of data with observations in the rows and variables in
the columns. Each table has the following properties:

* `name` (required): the table's name. Used to match the table to the database
  relation of the same name and to refer to it from `relationships`. Must be
  non-empty and unique within the dictionary.
* `label`: an optional human-facing display name, free of the identifier
  constraints on `name`.
* `description`: a human-readable description of the table. May contain
  markdown, and is usually a few sentences or a paragraph. A good description
  answers two questions:
    * **What's the grain?** What does a row represent? (e.g. "each row is a
      food item", "each row is one patient visit").
    * **What's the population?** What's been included or filtered out to
      produce this dataset? (e.g. "only completed orders from 2020 onwards",
      "excludes test accounts").
* `details`: additional information about the table. This is the place for
  "here be dragons": assumptions baked into the data, known weak spots,
  surprising calculations, and known problems. Also covers how the data was
  collected or constructed. Can be any length.
* `typedef`: table-scoped type aliases, same shape as the top-level
  [`typedef`](#typedefs). A name here shadows the same global name for this
  table's columns.
* `columns` (required): an ordered list of column metadata.

For example:

```yaml
tables:
  - name: trades
    description: >
      Each row is one executed trade. Includes all venues;
      excludes cancelled and test orders.
    typedef:
      side: ENUM('buy', 'sell')
    columns:
      - name: trade_id
        type: BIGINT
        constraints: [primary_key]
        description: Unique identifier for the trade.
      - name: side
        type: side
        description: Whether the trade bought or sold the instrument.
      - name: price
        type: money
        units: USD
        description: Execution price.
      - name: executed_at
        type: TIMESTAMP WITH TIME ZONE
        description: When the trade executed.
```

### Columns

Each entry in the `columns` list is a column descriptor. Columns are matched to
the database by `name`, so the order in which you list them does not need to
match the column order in the data.

Each descriptor has the following properties:

* `name` (required): column name. Used to match the descriptor to a column in
  the database (case-insensitively, like table names — ASCII folding). Must be
  non-empty and unique within a table.
* `label`: an optional human-facing display name.
* `type`: the column's data type — a native DuckDB type expression or the name
  of a `typedef` alias (see [Types](#types)). Optional — see below.
* `constraints`: a list of column-level constraints (see
  [Column constraints](#column-constraints)).
* `display`: controls whether the column should appear in user-facing output
  (see [Display](#display)).
* `description`: a human-readable description of the column. Can use markdown.
* `details`: additional information about the column, e.g. how it was computed
  or edge cases to watch out for. Can be any length.
* `values`, `range`, `examples`: representative values (see
  [Representative values](#representative-values)).
* `units`: the unit of measurement; only meaningful on a numeric column.
* `time_zone`: the time zone; only meaningful on a timestamp column (see
  [Time zones](#time-zones)).

A column may also be listed with only its `name` and no `type`. This
acknowledges the column without describing it, and you should use it for
columns that you don't care about but don't want flagged as undocumented. Such
a column makes no claim about its contents, so its type is never checked — but
it must still exist in the database.

#### Description & details

The `description` and `details` are free text fields that humans and agents
can use to jot down important notes. The `description` should be short,
typically a few sentences or at most a paragraph, and will be displayed in
user interfaces. The `details` can be any length, and is a good place to
carefully record all the details of the table.

#### Display

The optional `display` property controls whether a column should appear in
user-facing output. Currently, the only supported value is `restricted`:

```yaml
- name: ssn
  type: VARCHAR
  display: restricted
  examples: [000-00-0000]
```

A restricted column must be excluded from default user interfaces and other
user-facing output, including tables, plots, and downloads. We can't guarantee
this protection, but we hope it will steer agents (and humans!) away from
showing it by default.

#### Types

A column's `type` is written in DuckDB's own type system: any
[native DuckDB type expression](https://duckdb.org/docs/current/sql/data_types/overview.html) —
`BIGINT`, `DECIMAL(12, 2)`, `VARCHAR`, `ENUM('buy', 'sell')`,
`STRUCT(city VARCHAR, postcode INTEGER)`, `INTEGER[]`, `FLOAT[768]`, and so
on — or the name of a [`typedef`](#typedefs) alias, which expands to one.

There is no coarse intermediate vocabulary: the dictionary records exactly the
type the database has, so validation can compare with full fidelity (struct
fields, enum values, decimal precision, array sizes), and generators can
produce accurate DDL and code from the dictionary. Validation canonicalizes
both sides through DuckDB itself — the declared type is instantiated in a
scratch in-memory database and both sides are read back with `DESCRIBE` — so
spelling differences that DuckDB considers equivalent (case, whitespace, type
shorthands) never produce a false mismatch. A type expression DuckDB rejects
is reported with DuckDB's own error at the column's `type:` (see M09 in
[Validation](validation.md)).

#### Representative values

A column can describe its contents with representative values, using one of
three properties:

* `values`: the allowed values for a categorical column. Can be a list
  (`[M, F, U]`) when values are self-explanatory, or a map
  (`{M: Male, F: Female, U: Unknown}`) when values need labels. The values
  themselves must be scalars (string, number, or boolean); in the map form the
  labels must be strings. A column whose type is an `ENUM` already lists its
  categories in its type, so `values` is for categorical columns stored as
  plain types (most often `VARCHAR`).
* `range`: a two-element list `[min, max]` giving the inclusive minimum and
  maximum *observed* in the column. It describes the data rather than
  constraining it. Only meaningful on an orderable type — numeric, `DATE`,
  `TIMESTAMP`, or `TIMESTAMP WITH TIME ZONE` — and each bound must match the
  column's type: numbers for numeric columns, ISO 8601 dates (`2024-01-31`)
  for `DATE`, zoneless datetimes (`2024-01-31T09:30:00`) for `TIMESTAMP`, and
  offset-carrying datetimes (`2024-01-31T09:30:00Z`) for
  `TIMESTAMP WITH TIME ZONE`. The minimum must not exceed the maximum.

    Either bound may be left open with negative infinity (`-.inf`) for the
    minimum or positive infinity (`.inf`) for the maximum. An open bound says
    the true extent is unknown or constantly moving, as in a daily export whose
    date column always runs up to the present. If you leave a bound open, make
    sure to describe the range in prose in the column's `description`.
* `examples`: a list of ~5 representative values from the column. A handful of
  concrete examples helps LLMs understand the column far better than a
  description alone — knowing that an id column holds `[1, 2, 3, 4, 5]` versus
  `[10000, 1235452, 234234]`. A good baseline is to select 5 evenly spaced
  values along the sorted unique values, and then add any particularly
  surprising values as you encounter them.

None of the three is required: a bare DuckDB type carries no intent (a `BIGINT`
may be a measure or an identifier), so the dictionary cannot demand a
particular representation. Validation instead rejects combinations that cannot
make sense — `range` on an unorderable type, any of the three on a `BOOLEAN`
(its values speak for themselves), `units` on a non-numeric column, `time_zone`
on a non-timestamp column. See
[the descriptive-key checks](validation.md#the-descriptive-key-checks-in-the-rich-format)
for the exact rules.

#### Time zones

A timestamp column can declare its `time_zone`, which says how to interpret its
values as moments in time. The value is either an
[IANA time zone name](https://en.wikipedia.org/wiki/List_of_tz_database_time_zones)
or the sentinel `naive`:

* A named zone — `UTC`, `America/New_York`, `Europe/Paris`, and so on — means
  the column records instants in time, displayed in that zone. `UTC` is the
  usual choice for timestamps stored as instants.
* `naive` means the column records wall-clock date-times with no associated
  zone, so the same value can refer to different instants in different places.
  Use it for local times whose offset is unknown or irrelevant.

A named zone is either `UTC` or an IANA `Area/Location` name whose `Area` is
one of `Africa`, `America`, `Antarctica`, `Arctic`, `Asia`, `Atlantic`,
`Australia`, `Europe`, `Indian`, `Pacific`, or `Etc` (e.g. `America/New_York`,
`Etc/GMT+5`). Validation checks this shape and the `Area` — enough to catch
ambiguous abbreviations like `PST` or `EST` — but does not check the full
location against a time zone database, so the accepted set doesn't go stale as
zones are added or renamed.

Time zones are only meaningful for timestamps, so `time_zone` is an error on
any other type. Omit `time_zone` when the zone is unknown or doesn't matter.

```yaml
- name: observed_at
  type: TIMESTAMP
  time_zone: UTC
  description: A running log; the newest timestamp advances with every export.
  range: [2020-01-01T00:00:00, .inf]
```

NB: a column that declares a named `time_zone` is typically a zoneless
`TIMESTAMP` whose instants are interpreted in that zone. Range bounds follow
the column's *type*: a `TIMESTAMP` column writes plain, zoneless date-times;
a `TIMESTAMP WITH TIME ZONE` column writes bounds that carry their own
offset.

#### Column constraints

The `constraints` property is a list of constraint names. The supported
constraints are:

* `primary_key`: the set of columns with the `primary_key` constraint uniquely
  identifies each row. Implies `required` and `unique`.
* `foreign_key`: the column references a primary key in another table (or in
  the current table, if a self-join). The specific relationship is defined in
  [`relationships`](#relationships) — an *equality* conjunct pairing the
  column with a `primary_key` column. `dbdict validate-data` checks the
  reference against the data (see D04 in [Validation](validation.md)).
* `required`: the column does not contain null/missing values.
* `unique`: the column's values are distinct (no duplicates). NULLs are
  excluded, per SQL `UNIQUE` semantics. `dbdict validate-data` checks this
  against the data (see D03 in [Validation](validation.md)).

## Relationships

`relationships` is a list of join descriptors. Each entry describes how two
tables are related.

* `join` (required): a join expression of the form
  `table1.column = table2.column`, or
  `table1.date >= table2.start AND table1.date <= table2.end`.
* `cardinality` (required): either `one-to-one`, `one-to-many`, or
  `many-to-one`. Describes the relationship from the left table to the right
  table in the join expression. `dbdict validate-data` checks the declared
  cardinality against the data (see D05 in [Validation](validation.md)).
* `description`: human-readable description of the relationship. Only needed if
  it's not clear from the context.
* `conflicts`: a list of column names that appear in both tables with different
  meanings. These fields would cause ambiguity in a join and may need to be
  renamed or dropped.

For example:

```yaml
relationships:
  - join: food.food_category_id = food_category.id
    cardinality: many-to-one
    conflicts: [description]
```

## Glossary

`glossary` is a map from term to definition. Each entry provides a
plain-language definition of a domain-specific term used in the table or column
descriptions, or is likely to be used by a domain expert working with this
data.

```yaml
glossary:
  foundation food: >
    A food whose nutrient and food component values are derived
    primarily by chemical analysis.
```

## Version

`version` records the version of the data this dictionary describes, so people
and tools can tell two snapshots of the data apart and know which one a given
dictionary goes with. (This is distinct from `$version`, which records the
version of the *spec* the document conforms to.)

`version` is optional. It's a map with exactly one of three keys, which names
both the kind of version and its value:

* `number`: a hand-curated version number with three dot-separated numeric
  components, optionally followed by a pre-release (`-…`) and/or build (`+…`)
  suffix, such as `1.2.0` or `1.2.0-rc.1`.
* `date`: a release date in ISO 8601 form (`YYYY-MM-DD`), such as
  `2024-01-31`, for data refreshed on a schedule.
* `hash`: an opaque identifier, such as `a1b2c3d`, derived from the data
  itself.

If you use a `number`, we recommend
[semantic versioning](https://datapackage.org/recipes/data-package-version/):
increment the first component for incompatible changes, the second for
backwards-compatible additions, and the third for backwards-compatible fixes.

The validator checks that exactly one key is present, that a `number` has three
dot-separated numeric components (with an optional suffix), and that a `date`
is a valid ISO 8601 date, but otherwise treats the version as opaque.

```yaml
version:
  date: 2024-01-31
```

## The legacy format (0.1.0) {#the-legacy-format-010}

`$version: 0.1.0` selects the original `data-dict.yaml` format. Everything
above about the `name`/`description`/`details` trio, name-only columns,
`display`, `constraints`, `relationships`, `glossary`, and `version` applies
unchanged; this section describes where the legacy format differs. There is no
`typedef`, no `label`, and no dictionary-level `source` — and the rules for
representative values change from optional to required (below).

### Legacy source

`source` lives on each *table*, not on the dictionary, and points at a Parquet
file:

```yaml
tables:
  - name: food
    source:
      parquet: inst/parquet/food.parquet
```

* `parquet`: path to a Parquet file. Relative paths are resolved relative to
  the dictionary file.

Parquet is the only source the legacy format can validate against. As at the
dictionary level, a table's `source` is optional for spec validation but
required by the metadata and data levels.

### Legacy types

Legacy types capture data at a level that makes sense for analysis, which is
typically coarser than the logical types of the underlying data:

* `number`: numeric values (integers or floating-point). Can be qualified with
  a measure in parentheses: `number(id)`, `number(ordinal)`, or
  `number(quantity)` — see [Measures](#measures) below.
* `string`: UTF-8 text strings.
* `boolean`: true/false values.
* `date`: calendar dates, written as ISO 8601 strings (`YYYY-MM-DD`, e.g.
  `2024-01-31`).
* `datetime`: date-times, written as ISO 8601 strings. Without a `time_zone`
  they carry an offset (e.g. `2024-01-31T09:30:00Z`); with a `time_zone`
  they're written zoneless and interpreted in that zone.
* `enum`: a column with repeated values from a known set. The allowed values
  are listed in the `values` property.

#### Measures

The `number` type can be qualified with a measure in parentheses that
classifies what operations are meaningful:

| Type | Can compare | Can average | Can sum | Examples |
|------------|-------------|-------------|---------|----------|
| `number(id)` | No | No | No | primary keys, foreign keys, codes |
| `number(ordinal)` | Yes | No | No | ranks, years, sequence numbers |
| `number(quantity)` | Yes | Yes | Yes | weights, counts, amounts |

A `number(quantity)` column can also declare its `units`; units are an error on
any other legacy type. `time_zone` is likewise reserved for `datetime` columns.

#### Legacy representative values

Where the rich format makes representative values optional, the legacy format
*requires* each typed column to carry exactly one of `values`, `range`, or
`examples`, determined by the column's type:

* `values` for `enum` columns.
* `range` for the ordered numeric and temporal types: `number(ordinal)`,
  `number(quantity)`, `date`, and `datetime`.
* `examples` for all other types: `string`, `number`, and `number(id)`.

`boolean` columns are the exception: they can only contain `true`, `false`, and
(if not `required`) `null`, so they carry none of the three. Each `range` and
`examples` value must match the column's declared type.
