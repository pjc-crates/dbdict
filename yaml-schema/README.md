# How yaml-schema works

A summary of [yaml-schema](https://github.com/yaml-schema/yaml-schema), reconstructed from the Cucumber feature files in this directory and Alistair Israel's [introductory post](https://medium.com/@alistairisrael/introducing-yaml-schema-e53773237651).

## What it is

`yaml-schema` is a YAML-native validator. The schema language is a near-clone of JSON Schema (Draft 2020-12), but the schema itself is written in YAML and the validator is implemented in Rust (shipped as the `ys` CLI and as a library) so it operates directly on the YAML AST without a JSON round-trip. The author's framing: existing JSON Schema validators in JS/Python were too slow and lost YAML-specific information when converting; this fills that gap.

The schema is *itself* YAML — both the schema and the instance share the same syntax.

## The bare basics

A schema is just a YAML document. Three special whole-document forms:

| Schema | Meaning |
|--------|---------|
| *(empty)* | Accept anything |
| `true` | Accept anything |
| `false` | Reject anything |

Otherwise, a schema is a mapping with constraint keywords. The most important is `type`.

```yaml
type: string         # accepts "abc", rejects 42
type: number         # accepts 42 and 3.14
type: integer        # accepts 42 and 1.0; rejects 3.14
type: boolean        # accepts true/false; rejects "true"
type: 'null'         # accepts null only (NOT 0, "", or false)
type: array
type: object
```

Unknown types error at schema-load time:
```
type: foo  →  Unsupported type: Expected ... but got: foo
```

`type` can also be a list — a union of permitted types — and other keywords (e.g. `minimum`, `minLength`) apply to whichever branch matches:

```yaml
type: [string, number]
minimum: 1           # applies when value is a number
minLength: 1         # applies when value is a string
```

## Strings

```yaml
type: string
minLength: 2         # Unicode codepoints, not UTF-8 bytes
maxLength: 3
pattern: "^[0-9]+$"  # ECMA-style regex
description: "..."   # annotation only
```

## Numbers

```yaml
type: number         # or integer
minimum: 0           # inclusive
maximum: 100         # inclusive
exclusiveMinimum: 0  # exclusive
exclusiveMaximum: 100
multipleOf: 10
enum: [1, 10, 100]
```

`integer` accepts floats whose fractional part is zero (`1.0` is fine; `3.14` is not).

## Arrays

```yaml
type: array
items:               # schema for every element
  type: number
minItems: 2
maxItems: 4
uniqueItems: true
```

**Tuple validation** with `prefixItems` — a per-position list of schemas:

```yaml
type: array
prefixItems:
  - type: number
  - type: string
  - enum: [Street, Avenue, Boulevard]
items: false         # disallow extras
# items: { type: string }  # OR: extras must match this schema
# items: omitted           # OR: any extras are allowed
```

Fewer items than `prefixItems` is always OK; what `items` controls is what happens *after* the prefix.

**Contains** asserts that *some* item matches (vs. `items` requiring *all*):

```yaml
type: array
contains: { type: number }
minContains: 2       # at least 2 must match (default 1)
maxContains: 3       # at most 3 may match
# minContains: 0  →  contains is satisfied even with zero matches
```

## Objects

YAML keys may be numbers or strings — both are treated as object keys.

```yaml
type: object
properties:                    # named properties; each value is a schema
  number: { type: number }
  street_name: { type: string }
required:                      # MUST be present and non-null
  - street_name
patternProperties:             # regex on key name → schema for value
  ^S_: { type: string }
  ^I_: { type: integer }
additionalProperties: false    # OR true (default), OR a schema
propertyNames:                 # constrain the *names* themselves
  pattern: "^[A-Za-z_][A-Za-z0-9_]*$"
minProperties: 2
maxProperties: 3
```

Important rules:

- By default, missing properties are allowed (use `required` to force them).
- A property explicitly set to `null` counts as **not present** for `required`.
- If a key matches both `properties` and a `patternProperties` regex, **both** schemas apply (intersected).
- `patternProperties` takes priority over `additionalProperties` when both could match.
- `$schema` keys in instance data are silently ignored.

### Dependencies between properties

```yaml
type: object
dependentRequired:             # if key X is present, also require Y, Z
  credit_card:
    - billing_address
dependentSchemas:              # if key X is present, also validate against this schema
  credit_card:
    type: object
    required: [billing_address]
```

`dependentRequired` is one-directional unless you declare both ways.

## Constants and enums

```yaml
const: "United States of America"   # exact equality (also works for arrays/objects)
enum: [red, amber, green]           # any-of these literal values
enum: [red, null, 42]               # untyped enum: heterogeneous values OK
```

## Composition

```yaml
allOf: [ ..., ... ]   # must satisfy ALL subschemas
anyOf: [ ..., ... ]   # at least one
oneOf: [ ..., ... ]   # exactly one (more than one match → fail)
not: { ... }          # the inverse
```

A common idiom — "null or a typed object":

```yaml
oneOf:
  - type: 'null'
  - type: object
    properties:
      name: { type: string }
    required: [name]
```

## Conditionals: if / then / else

```yaml
type: object
properties:
  country: { enum: [USA, Canada] }
if:
  properties:
    country: { const: USA }
then:
  properties:
    postal_code: { pattern: '[0-9]{5}(-[0-9]{4})?' }
else:
  properties:
    postal_code: { pattern: '[A-Z][0-9][A-Z] [0-9][A-Z][0-9]' }
```

Multi-way branching is done by wrapping individual `if`/`then` pairs in `allOf`.

## Formats

`format` annotates a string with a semantic type. Recognized formats are validated; unknown formats are accepted (annotation-only):

```
date          date-time     time          duration
email         hostname      uri           uuid
ipv4          ipv6          json-pointer  regex
```

`format` composes with other string constraints (e.g. `minLength` still applies).

## Unevaluated properties / items

These keywords look at *what siblings already validated* and reject anything they didn't cover. Useful for closing off open schemas built from `allOf`/`anyOf`:

```yaml
allOf:
  - properties:
      a: { type: string }
  - unevaluatedProperties: false   # rejects { a: x, b: y } — `b` was never evaluated
```

`unevaluatedItems` works the same way for arrays after `prefixItems`/`items`.

## References

```yaml
$defs:
  name:
    type: string
type: object
properties:
  name: { $ref: "#/$defs/name" }
```

- Local refs use JSON Pointer fragments (`#/$defs/name`).
- Remote refs (full URI + fragment) are supported, e.g. `https://yaml-schema.net/yaml-schema.yaml#/$defs/valid_types`.
- Direct or indirect circular `$ref` chains are detected and reported (`Circular $ref detected: ...`).

## CLI

`ys` is the binary.

```bash
ys version
ys -f schema.yaml instance.yaml          # explicit schema
ys instance.yaml                         # uses top-level $schema in the file
ys --json -f schema.yaml instance.yaml   # JSON-formatted error output
```

Exit code `0` on valid, `1` on invalid. Error messages carry source positions:

```
[1:6] .foo: Expected a string, but got: 42 (int)
[2:6] .bar: Expected a number, but got: "I'm a string" (string)
```

## Mental model

If you know JSON Schema 2020-12, you know yaml-schema — almost everything carries over (`type`, `properties`, `required`, `items`, `prefixItems`, `allOf`/`anyOf`/`oneOf`/`not`, `if`/`then`/`else`, `$ref`/`$defs`, `unevaluatedProperties`, `dependentRequired`/`dependentSchemas`, format annotations). The differences are presentation and provenance: the schema is YAML, the validator is Rust, error messages quote YAML source positions, and YAML-isms (numeric keys are strings; explicit `null` ≠ absent) are honored natively rather than washed out by a JSON conversion.
