---
title: "Which ids the engine's own fields, indexes and other non-type objects take"
pick: one
options:
  fixed-table:
    title: "catalog.md §4.6 lists them per id space, below 1024, per catalog version"
  declaration-order:
    title: "Assigned below 1024 by the order of §1.2 applied to the engine's declarations, as if they were a log"
  none:
    title: "They have no catalog rows: catalog::fields and catalog::indexes list only the log's objects"
---
Raised by `id-assignment-gaps`. catalog.md §4.6 fixes ids below 1024
for engine-defined types, constructors and relation variables, which
share one id space. The engine's own fields, indexes (keys and
uniques), defaults and named sets, each in an id space of its own,
have none, though §1.2 says every schema object has a catalog id and
the bootstrap catalog writes rows for them.

## spec/ids/engine-objects/fixed-table

§4.6 gains a table per id space (fields, indexes) for the prelude, then
`catalog`, then `sys`, fixed per edition and catalog version like the
type table. Long, but every tool reads one table.

## spec/ids/engine-objects/declaration-order

Every space starts at 0 for the engine. The engine's declarations, in
the order catalog.md gives them, create objects as §1.2's operators do
(`add type`, then `add relation`), each taking the next id of its
space. A catalog version that adds an object takes the next unused id.
The table of §4.6 stays as it is, and its type ids already follow this
order.

## spec/ids/engine-objects/none

Only objects of the log have field, index and default rows. A reader
of `catalog::fields` cannot see the catalog's own fields, and
`sys::field_stats` cannot name them.
