# Validation

## Three levels of validation

Validation happens at three levels, each a strict superset of the one before it:

* Validating the **spec** checks that the dictionary file itself conforms to the dbdict spec — that it is well-formed and internally consistent, with valid types, foreign keys that have matching relationships, joins that parse, and so on. These checks have an unambiguous right answer, so most are errors. This level looks only at the dictionary file, never at the data. This is performed by `dbdict validate-spec`.

* Validating the **metadata** checks that the data's column names and types match the dictionary. It reads only the data's metadata (for example, a Parquet file's footer), not its values, so it stays cheap. This is performed by `dbdict validate-meta`.

* Validating the **data** checks that the data's values match the dictionary — that required columns have no nulls, and so on. This is the only level that reads the data itself, so it can be expensive, depending on the data source. This is performed by `dbdict validate-data`.

The last two levels compare the dictionary against the data (or equivalently, the data against the dictionary). When they disagree, we can't tell which side needs to change. If you're creating the dictionary as you learn about the data, then you might need to change the dictionary. If you're using the dictionary to validate a dataset, there might be an upstream issue that you need to resolve.

The metadata and data levels locate each table's data through its [`source`](spec.md#source). In the legacy (0.1.0) format they read the file each table's `source.parquet` points at, resolved relative to the dictionary file. In the rich (0.2.0) format the whole dictionary describes one DuckDB database, named by the dictionary-level `source.duckdb.file` (also resolved relative to the dictionary): `validate-meta` instantiates each typed column in a scratch in-memory DuckDB (applying the dictionary's `typedef:` aliases), `DESCRIBE`s both the scratch columns and the real database, and compares the canonical types exactly. Identifiers are matched case-insensitively, as DuckDB's are. Either way, every table in the dictionary is validated in a single run, and a problem in one table is reported against that table without stopping the others from being checked. (The rich format's *data* level is not built yet; `validate-data` on a rich dictionary reports one "not yet supported" pre-flight failure.)

Each level implies the ones before it: validating the metadata validates the spec first, and validating the data validates both the spec and the metadata first. Validating the spec and metadata are cheap, so they can be run continually while you edit the dictionary; validating the data adds a full scan and get more expensive as the size of the data increases.

Each check has a code prefixed by its level: spec checks are `S01`, `S02`, …; metadata checks `M01`, …; data checks `D01`, …. Severity is independent of level — any level can raise errors or warnings.

## Errors vs warnings

A validator reports two severities of problem: **errors** and **warnings**. The distinction is about urgency, not importance.

* An **error** means the dictionary is invalid or there's a critical mismatch between the data and dictionary. Errors will cause a production pipeline to fail, and you must fix them immediately.

* A **warning** means the dictionary is usable but the data and dictionary may have drifted apart. Warnings will not cause a production pipeline to fail, but if you're actively working on the project you should make sure to fix them.

## Spec-validation checks

When validating the spec, each problem with the dictionary is one of:

* **Unresolved foreign key** (S01, error): a `foreign_key` column has no `relationships` entry pointing it at a `primary_key` column.
* **Unknown table** (S02, error): a relationship references a table that is not defined in `tables`.
* **Unknown column** (S03, error): a relationship references a column that does not exist on its table.
* **Invalid join** (S04, error): a `join` expression fails to parse, or references neither one (self-join) nor two tables.
* **Unresolved conflict column** (S05, error): a name in `conflicts` is not a column on both sides of the join.
* **Inconsistent cardinality** (S06, error): the declared cardinality is inconsistent with the constraints on the joined columns (e.g. `one-to-many` whose "one" side is not `primary_key` or `unique`).
* **Wrong representation key** (S07, error): a column's data representation key is absent or wrong for its type (`enum` → `values`; `number(ordinal)`, `number(quantity)`, `date`, `datetime` → `range`; otherwise → `examples`). A `boolean` column must carry none of `values`, `range`, or `examples`.
* **Units without quantity** (S08, error): a column has `units` but its type is not `number(quantity)`.
* **Missing `$learn_more`** (S09, warning): the document omits the recommended `$learn_more` key.
* **Duplicate name** (S10, error): two column descriptors within the same table share a `name`, or two table descriptors within the dictionary share a `name`. In the rich (0.2.0) format names are compared ASCII-case-insensitively, because DuckDB identifiers fold case — `Food` and `food` cannot both exist in the database. Legacy (0.1.0) names are compared exactly (parquet column names are case-sensitive).
* **Empty name** (S11, error): a table name or a column `name` is empty.
* **Wrong value type** (S12, error): a value in `range` or `examples` does not match the column's `type` — a number type wants numbers; `string` wants strings; `date` wants an ISO 8601 date (e.g. `2024-01-31`); `datetime` wants an ISO 8601 datetime, with an offset (e.g. `2024-01-31T09:30:00Z`) unless the column has a `time_zone`, in which case it's zoneless (e.g. `2024-01-31T09:30:00`). A `range` bound may instead be `-.inf` (minimum) or `.inf` (maximum) to leave that end open, on any range type.
* **Descending range** (S13, error): a `range`'s minimum is greater than its maximum. An open bound counts as ordered only in its own place — `-.inf` as the minimum and `.inf` as the maximum; `.inf` as a minimum or `-.inf` as a maximum runs backwards.
* **Time zone without datetime** (S14, error): a column has `time_zone` but its type is not `datetime`.
* **Malformed time zone** (S15, error): a `time_zone` is not `naive`, `UTC`, or an IANA `Area/Location` name with a known area. The shape is checked, not the full tz database, so the accepted set doesn't go stale as zones are added or renamed.
* **Misplaced single-table description** (S16, warning): a dictionary with exactly one table carries `description` or `details` on that table; for a single-table dictionary these belong at the top level.
* **Malformed version** (S17, error): the top-level `version` does not give exactly one of `number`, `date`, or `hash`; its `number` is not three dot-separated numeric components (`MAJOR.MINOR.PATCH`) with an optional pre-release/build suffix; or its `date` is not a valid ISO 8601 date (`YYYY-MM-DD`).
* **Non-string typedef name** (S18, error, rich format only): a `typedef:` key is not a string (an unquoted `123:` or `true:` parses as a number or boolean). The schema constrains typedef *values* only, so without this check the alias would silently vanish from the dictionary.

(An `enum`'s `values` are constrained structurally by the schema rather than by an `S` check: each value must be a scalar, and in the map form each label must be a string. The `version` map's allowed keys and their value types are likewise structural; S17 covers only the semantics the schema can't express.)

### The descriptive-key checks in the rich format

S07, S08, and S12–S14 are written above against the legacy coarse types. For rich (0.2.0) documents the same rules are reworked against the column's *canonicalized* DuckDB type, and because canonicalization needs the scratch database, they run during `validate-meta` rather than `validate-spec` (keeping their `S` codes — the rules are the same, only the level they can run at moved):

* Nothing is **required**: a DuckDB type carries no measure/id intent (`BIGINT` may be a quantity or an identifier), so a column with no descriptive keys is always fine, and `values` is welcome on any non-boolean column (a `VARCHAR` category column is legitimate).
* **S07**: `range` is rejected on types that aren't orderable — `ENUM(...)` (the type lists its categories), `BOOLEAN`, and composite/other types (structs, arrays, maps, `VARCHAR`, …). A `BOOLEAN` column still carries none of `values`, `range`, or `examples`.
* **S08**: `units` requires a numeric type (integers, floats, `DECIMAL`).
* **S14**: `time_zone` requires a timestamp type (`TIMESTAMP` at any precision, or `TIMESTAMP WITH TIME ZONE`).
* **S12/S13**: `range` bounds must match the column's category — numbers for numerics, ISO 8601 dates for `DATE`, zoneless datetimes for `TIMESTAMP`, offset-carrying datetimes for `TIMESTAMP WITH TIME ZONE` — and order min-to-max, with `-.inf`/`.inf` leaving an end open as in the legacy rules. Only `range` is type-checked in rich mode: `examples` and `values` are illustrative/categorical documentation, not type claims (the exact type round-trip in M01 already pins the column's type), so their contents are not checked against the column type as legacy S12 does.

## Metadata-validation checks

When validating the data's metadata against the dictionary, each column mismatch is one of:

* **Type mismatch** (M01, error): a column's declared type is incompatible with the data. In the rich format the comparison is exact: the dictionary column's canonicalized DuckDB type (aliases expanded) must equal the database column's, byte for byte.
* **Missing column** (M02, error): a column the dictionary describes is absent from the data. This applies even to columns listed by name only — listing a column that doesn't exist is an error.
* **Undocumented column** (M03, warning): a column present in the data that the dictionary does not describe. This is a warning, not an error: if a production pipeline adds a column, validation should not fail, but you should document it (or at least list it by name) next time you touch the dictionary.
* **Missing source** (M04, error): a table validated against data does not declare a `source` (legacy), or a rich dictionary declares no dictionary-level `source`. `source` is optional at the spec level but required here, so a validated dictionary always records where its data comes from.
* **Unreadable source** (M05, error): a `source` is declared but its data can't be read — the `source.parquet` file is absent or not readable Parquet (legacy), or the DuckDB database can't be opened (rich). The path is resolved relative to the dictionary file.
* **Missing table** (M06, error, rich format only): a table the dictionary describes is absent from the database.
* **Undocumented table** (M07, warning, rich format only): a table (or view) present in the database that the dictionary does not describe. A warning for the same reason as M03. Skipped when validating a single named table.
* **Rejected typedef** (M08, error, rich format only): DuckDB rejected a `typedef:` when the dictionary was instantiated in the scratch database — an unknown or cyclic reference, or a malformed type expression. Cycles surface here naturally: typedefs are created with retry-until-stall, so the stalled leftovers are exactly the cyclic or dangling group, each reported with DuckDB's own error at its definition.
* **Rejected column type** (M09, error, rich format only): DuckDB rejected a column's `type:` expression. The column has no canonical form, so it is skipped by the M01 comparison; the rest of its table is still checked.

## Data-validation checks

When validating the data's values against the dictionary, each column mismatch is one of:

* **Nulls in a required column** (D01, error): a `required` or `primary_key` column contains nulls.
