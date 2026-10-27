---
title: "What happens to a row whose stored form does not fit the engine's row limit"
pick: one
options:
  refuse-at-write:
    title: "Refused at the statement with RowTooLarge naming the relation, constructor, size and limit; a migration declaring a constructor whose fixed part can never fit is refused"
  field-limit:
    title: "A field limit stated independently of page size, small enough for the smallest page"
  continue:
    title: "Rows continue across pages, so no row is too large"
---
Raised by `spec-row-too-large`. The heap refuses a row whose fixed part
does not fit on a page, and no error sum carries the refusal. The limit
depends on the page size the database chooses at creation. Two schemas
show it: a record of 300 `Decimal` fields, whose fixed part exceeds a
4 KiB page, and a record of 400 `Text` fields, whose out-of-line
pointers alone exceed a page.

## spec/row-too-large/refuse-at-write

A row whose stored form does not fit the engine's row limit is refused
at the statement with `Rejected { reason: RowTooLarge { relation, ctor,
size, limit } }`. A migration declaring a constructor whose fixed part
can never fit under the open database's limit is refused with
`SchemaRefused` naming the constructor. The limit is the engine's and
may differ between databases of different page sizes, so a schema can
be valid in one database and refused in another; a checked schema
never fails every insert, since the migration refuses the constructor
that would.

## spec/row-too-large/field-limit

A limit on fields stated independently of page size. Portable, but
it must be sized for the smallest page, which forbids wide rows a
large page could hold, and Text and Bytes fields still need a pointer
each.

## spec/row-too-large/continue

Rows continue across pages. Removes the limit, at the cost of a
multi-page row format in the heap, its WAL images and its vacuum.
