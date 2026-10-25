---
title: "What insert ... rejects into r means"
pick: one
options:
  per-row:
    title: "Each source row's conversion is tried; rows that convert are inserted, rows that fail go to r with their position and error; load is fresh per statement"
  interop-only:
    title: "Defined only by interop's description of loads"
---
Raised by the implementability review of docs/spec/. `rejects into`
was in the grammar and interop §4.5 but had no meaning in writes.md or
the kernel.

## spec/writes/rejects/per-row

Each source row's conversion is tried. Rows that convert are inserted
into the target; rows that fail are inserted into `r` with their
position in the source's stated order and the error. `load` is a fresh
value per statement. Stated declaratively in writes.md and semantics
§6, with interop §4.5 consistent with it.

## spec/writes/rejects/interop-only

Left to interop's prose about loads. The form has no denotation, so
two implementations may disagree on which rows land where.
