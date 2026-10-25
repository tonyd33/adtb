---
title: Where design rationale lives
pick: one
options:
  in-spec:
    title: Informative rationale sections beside the normative text
    links:
      - {rel: prior-art, to: prior/third-manifesto}
  design-space:
    title: The quandary space is the record; the spec states only rules
  none:
    title: No rationale is kept with the spec
---
Rulings in `points/` carry reasons. The spec reader needs the rule;
the person changing it needs the reason.

## project/spec/rationale/in-spec

The Third Manifesto's book interleaves prescriptions with long
discussion; Ada's Rationale is a separate companion (unverified).

## project/spec/rationale/design-space

Each normative section names the questions it answers; reasons live in
the rulings.

```
<!-- spec/lang.md, section Match: answers query/compile-time, lang/wildcard/scalars-only -->
```

## project/spec/rationale/none

Rules only.
