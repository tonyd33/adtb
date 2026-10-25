---
title: Whether data carries information-flow labels
pick: one
options:
  none:
    title: No labels; access control only
  static:
    title: Labels on constructors and fields, propagated through query types and checked at registration
  floating:
    title: A transaction's label rises with what it reads and bounds what it may write
    links:
      - {rel: prior-art, to: prior/security-lio}
      - {rel: prior-art, to: prior/security-hails}
  faceted:
    title: Sensitive values hold a secret and a public facet, resolved for the viewer at output
    links:
      - {rel: prior-art, to: prior/security-jacqueline}
links:
  - {rel: rivals, to: security/policy/predicate}
---
Access control decides what a query may read; information flow decides
where what it read may go: into results for whom, into which written
rows.

## security/ifc/none

Grants and policies decide reads; nothing follows data afterwards.

## security/ifc/static

```
type Patient = { id: Uuid, name: Text @{clinic}, diagnosis: Text @{patient, doctor} }
query count_by_diagnosis () -> Set { d: Text @{patient, doctor}, n: I64 } = ...
```

Jif-style (unverified) labels on types. Every application query is
registered, so the whole program the database serves is checked
statically; a recheck on schema change also rechecks flows.

## security/ifc/floating

LIO and Hails: reading a row labelled `alice` raises the transaction's
label; a later write to a public relation is refused. Hails computes
document and field labels by pure functions of the document, and its
collection label also guards key values.

## security/ifc/faceted

```
type Event = { name: Faceted Text }   -- Secret Text | Public Text, chosen per viewer
```

Jacqueline stores each facet as a row tagged with `jvars`; here a facet
is a constructor and facets are rows of constructor relations.
