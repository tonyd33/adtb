---
title: "Whether prelude functions have catalog ids and rows"
pick: one
options:
  none:
    title: "No: the prelude is not part of a schema, so its functions have no id and no row"
  fixed:
    title: "Yes: each takes a fixed id below 1024 and a catalog::functions row"
---
Raised by the catalog id work. §4.6 fixes ids for the engine's types,
constructors, relation variables, fields and indexes, and says nothing
of `count`, `read_csv` and the other prelude functions.

## spec/ids/prelude-fns/none

The prelude is not part of a schema. Its functions have no catalog id
and no row of `catalog::functions`; operators and plans name them by
name and edition.

## spec/ids/prelude-fns/fixed

Each prelude function takes a fixed id per edition and a row of kind
`Host`. Adds a table to §4.6 that changes with every edition.
