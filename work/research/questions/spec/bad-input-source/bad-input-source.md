---
title: "What BadInput's source and line hold when no single source record failed"
pick: one
options:
  empty:
    title: "Defined only for a statement over one source; otherwise source is \"\" and line is 0"
  option:
    title: "source and line become Option fields"
  first-generator:
    title: "They name the record of the first generator"
---
Raised by the reference interpreter's writes. `BadInput { source,
line, detail }` names a failing record, but a `reject` can be reached
by an insert whose source is any query, with no one record to name.

## spec/bad-input-source/empty

`source` and `line` name the failing record when the statement's
source is a comprehension with a single generator over one source
relation or external source, the shape `rejects into` requires
(writes.md §2.5). Otherwise `source` is `""` and `line` is `0`.

## spec/bad-input-source/option

`source: Option Text` and `line: Option I64`. Precise, but changes a
prelude constructor of the edition and makes every reader match.

## spec/bad-input-source/first-generator

The record of the first generator of the binding that failed. Defined
everywhere, but names a record that may not be the one at fault.
