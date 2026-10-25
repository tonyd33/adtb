---
title: What wire tag a constructor of a query result sum gets
pick: one
raised-by: [model/sum-results]
options:
  declared:
    title: A result sum must be a declared type, so its constructors have catalog ids
  query-scoped:
    title: Anonymous result constructors are numbered within the registered query signature
---
A match whose arms build different shapes returns a sum. Stored
constructors have never-reused catalog ids; anonymous ones have none.

## surface/declared

```
type LoginKey = ByEmail { email: Text } | ByGoogle { gid: Text }
query login_keys () -> Set LoginKey = ...
```

## surface/query-scoped

The signature `-> Set (ByEmail { email: Text } | ByGoogle { gid: Text })`
numbers its constructors in order; a changed signature is a new query
version.
