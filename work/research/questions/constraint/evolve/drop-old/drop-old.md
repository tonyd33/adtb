---
title: Readers of the old schema assume a constraint that was dropped
pick: one
raised-by: [constraint/evolve/icmo]
options:
  ignore:
    title: Old readers see violating rows unfiltered
    links:
      - {rel: prior-art, to: prior/constraint-prism-icmo}
  filter:
    title: Old readers see only rows satisfying the dropped constraint
    links:
      - {rel: prior-art, to: prior/constraint-prism-icmo}
  readers-first:
    title: A drop takes effect only once no registered reader relies on it
    links:
      - {rel: variant-of, to: evolution/old-readers/readers-first}
---
Dropping a constraint widens the data. A query compiled against a
refinement type or an FD the drop removed reads rows it was typed to
exclude. PRISM++ chooses a policy for the inverse ICMO that serves old
queries.

## constraint/evolve/drop-old/ignore

PRISM++ `IGNORE`.

## constraint/evolve/drop-old/filter

PRISM++ `ENFORCE`: the old version's queries gain the constraint as a
`where`, so a violating row is invisible to them.

## constraint/evolve/drop-old/readers-first

As constructors are added only after readers can type them, a
constraint is dropped only after every registered query recompiles
without it.
