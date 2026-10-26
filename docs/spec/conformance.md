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

A directive starts in the first column, and its body is the lines
below it indented by two spaces; a body may hold blank lines. A line
starting with `#` between directives is a comment. A query's rows
follow `----` and end at a blank line, a comment or the next
directive; a query with no rows is `----` followed by a blank line.
Without `schema`, the database holds only the prelude.

- A set result is compared as a set, and its rows are written in
  printed order ([interop.md §1.1](interop.md#11-one-syntax));
  `rowsort` lifts the order.
- A result ordered by `order by` is compared in order, and as a
  multiset under `rowsort`. Elements tied on the ordering are
  delivered in any order, so a script orders them determinately or
  uses `rowsort`.
- A result that is not a set is one row.
- `error C` runs its statements in order: each but the last succeeds,
  and the last is refused with `C`. Its text is one ad hoc text, so
  none of its statements commits
  ([queries.md §16](queries.md#16-ad-hoc-statements)).
- A `query` that changes the database differs from its script.

The suite holds one directory per part of this specification, named
after it (`model`, `queries`), and one script per file, with the
extension `.test`.

Rulings: project/spec/test-format/script, spec/script-format/indented

## 3. Running a test

A runner executes each script against a fresh database, once against
the engine and once against the reference interpreter, and reports
every directive whose outcome differs from the script. Outcomes are
compared as [semantics.md §1.3](semantics.md#13-conformance) states:
up to the freedoms it lists, with float aggregates within its bound,
and every other value exactly. An implementation conforms only if no directive of
any test of the suite differs.

Rulings: project/spec/conformance/normative-suite, project/oracle/float/tolerance
