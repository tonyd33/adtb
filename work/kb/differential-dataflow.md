# Differential Dataflow

kind: paper
home: https://www.cidrdb.org/cidr2013/Papers/CIDR13_Paper111.pdf

## Model

- McSherry, Murray, Isaacs, Isard, "Differential dataflow", CIDR 2013, Microsoft Research; prototype in Naiad. [V 2026-10-25] https://www.cidrdb.org/cidr2013/Papers/CIDR13_Paper111.pdf
- State varies over a partially ordered set of versions (multi-dimensional logical timestamps, e.g. (input epoch, iteration)), not a total order; differences are retained in an indexed structure instead of consolidated into a current state, so they can be reused across input changes and loop iterations. [V 2026-10-25] same
- Supports arbitrarily nested iteration on changing input, e.g. incrementally maintained strongly connected components. [V 2026-10-25] same
