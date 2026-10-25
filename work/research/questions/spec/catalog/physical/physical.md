---
title: "What catalog statistics, waits and findings are keyed by"
pick: one
options:
  per-ctor-relation:
    title: "Keyed by constructor relation; fields naming physical detail are Text with implementation-defined content"
  per-heap:
    title: "Keyed by heap and page, with fixed physical fields"
---
Raised by the tier audit. Catalog relations named heaps, pages, page
ids, tids, latches, vacuum phases and B-link invariants as typed
fields, while semantics §4.2 says no program observes heaps or pages.

## spec/catalog/physical/per-ctor-relation

Statistics, horizons, waits, conflicts, vacuum progress and verify
findings are keyed by constructor relation, never by heap or page. A
field keeps a fixed meaning only when its meaning is independent of
representation (row counts; versions no snapshot can see). A field
naming physical detail becomes `Text` whose content is
implementation-defined. `VacuumPhase` and the B-link invariants leave
the catalog.

## spec/catalog/physical/per-heap

Keep heap-keyed rows with typed physical fields. Operators see more
structure, but the catalog fixes the storage layout as a promise and
contradicts semantics §4.2.
