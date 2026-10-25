---
title: "What docs/spec states, and where the rest of the design lives"
pick: one
options:
  three-tiers:
    title: "Three tiers: docs/spec what users observe, docs/design contracts between subsystems, the rest with the code"
  annexes:
    title: "One normative spec plus informative annexes for the engine and the implementation"
---
Raised by the tier audit of docs/spec. Normative parts stated
mechanism (heaps, pages, probes, the planner) beside guarantees, and
two informative parts, engine.md and implementation.md, held tier-1
facts with no normative home.

## spec/tiers/three-tiers

docs/spec is tier 1: what users and clients observe, stated
declaratively. docs/design is tier 2: contracts between subsystems; a
subsystem relies only on what its counterpart's design file states.
Tier 3 lives with the code; until code exists it sits under "Notes for
the implementation" in a design file, non-binding. A rule belongs to
the lowest tier that can observe it.

```text
semantics.md §7.2   only rows holding Paid take part in unique (status.Paid.ref)   tier 1
mvcc.md §7          the uniqueness outcome table per index                         tier 2
indexes.md §8       B-link node layout                                             tier 3
```

## spec/tiers/annexes

docs/spec stays one tree: normative parts plus informative annexes
(engine.md, implementation.md). Fewer files, but annex text is read as
a promise, and the spec couples to one implementation's structure.
