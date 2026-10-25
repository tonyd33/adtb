---
title: Whether a query result can be a relation-level sum
pick: one
raised-by: [sum/relation]
options:
  sum-results:
    title: A query result may be a sum whose constructors are its arms
  flat-results:
    title: A query result is a relation of records and a tag is a value-level sum field
---
Stored relations may be sums. Whether the algebra is closed over that:
can a query or view produce a relation-level sum that another query
matches with exhaustiveness and constructor relations.

## model/sum-results

An arm may construct a result constructor; the result is the disjoint
union of arms with the tags kept, matchable downstream.

```
fn logins () -> Set (Local { id: Key identities, email: Text }
                   | Remote { id: Key identities, provider: Text }) =
  from i in identities select match i
    | EmailPass e -> Local  { id: key e, email: e.email }
    | Google g    -> Remote { id: key g, provider: "google" }

from l in logins().Remote select l.provider
```

Views over sums are then sums, and `match` on a view is checked like
`match` on a stored sum.

## model/flat-results

Results are relations of records. A tag survives only as a field of a
value-level sum type.

```
from i in identities select { id: key i, kind: match i | EmailPass _ -> Local | Google _ -> Remote }
```

A view is never itself a sum, so constructor relations and ctor-set
scopes exist only for stored relations.
