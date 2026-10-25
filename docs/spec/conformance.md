# Conformance suite

This part defines the conformance suite: what it is, the format of its
tests, and how a test is run and judged. Which source wins where
sources disagree is stated in [README.md](README.md#conformance).

## 1. The suite

Each edition is released as prose, the reference interpreter and the
conformance suite together. The suite is normative. The reference
interpreter evaluates the semantics of [semantics.md](semantics.md)
directly; it answers what any query over any database should return.

Rulings: project/spec/conformance/normative-suite, project/spec/shape/executable

## 2. Tests

A test is a script of statements and queries with expected rows:

```text
schema
  module test
  type Login { id: Key logins } = Password { email: Text } | Social { sub: Text }
  relation logins : Login key id generate sequence
run
  transact
    insert logins.Social { sub: "s1" }
query rowsort
  from l in logins select match l | Password p -> p.email | Social s -> s.sub
----
"s1"
error Invalid
  from l in logins select match l | Social s -> s.sub
```

| Directive | Meaning |
| --- | --- |
| `schema` | Declarations applied to an empty database as one migration |
| `run` | Statements executed in order; each must succeed |
| `query` | A query whose result rows follow `----`, one per line in textual value syntax; `rowsort` compares them as a set |
| `error C` | A statement or query that must be refused with the reason constructor `C` of [catalog.md](catalog.md#4-error-sums) (here `Invalid`: the text fails to type-check) |

Values are written in the textual syntax of
[interop.md](interop.md#1-textual-value-syntax).

Rulings: project/spec/test-format/script

## 3. Running a test

A runner executes each script against a fresh database, once against
the engine and once against the reference interpreter, and reports
every directive whose outcome differs from the script. Outcomes are
compared as [semantics.md §1.3](semantics.md#13-conformance) states:
up to the freedoms it lists, with float aggregates within its bound,
and every other value exactly. An implementation conforms only if no directive of
any test of the suite differs.

Rulings: project/spec/conformance/normative-suite, project/oracle/float/tolerance
