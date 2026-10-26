---
title: "How a conformance script lays out directives, bodies and rows"
pick: one
options:
  indented:
    title: "Directives in the first column, bodies indented two spaces, rows after ---- up to a blank line, comment or directive; sets in printed order unless rowsort"
  sqllogictest:
    title: "sqllogictest's layout: records separated by blank lines, a type string per query"
---
Raised by the conformance runner. conformance.md §2 shows one script
and names the directives, but not where a body ends, where rows end,
how comments are written, or what `query` without `rowsort` compares
for a set result, which has no order.

## spec/script-format/indented

A directive starts in the first column and its body is the lines below
it indented by two spaces, so a body may hold blank lines. A query's
rows follow `----` and end at a blank line, a `#` comment or the next
directive. A set result is compared as a set and its rows are written
in printed order (interop.md §1.1),
which `rowsort` lifts; an `order by` result is compared in order, and
as a multiset under `rowsort`. A result that is not a set is one row.
`error C` runs its statements in order and the last is refused with
`C`; as one ad hoc text, none of them commits. A query that changes the database differs. The suite is one
directory per spec part, one `.test` file per script.

## spec/script-format/sqllogictest

Records separated by blank lines, a type string after `query` naming
each column's type. The values here are typed by the query, and rows
are single values in the textual syntax, so a type string adds
nothing, and blank-line separation forbids blank lines in a body.
