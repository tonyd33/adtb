---
title: How a client asks for filters and sorts chosen at run time
pick: some
options:
  option-params:
    title: Option parameters whose None disables a filter, specialised at plan time
  variants:
    title: A query declares optional clauses and every combination is checked at registration
  adhoc-only:
    title: Dynamic queries go through the ad hoc role
---
A search screen with N optional filters and a user-chosen sort cannot
be a builder (defeated) and should not be 2^N registered queries.

## lang/option-params

```
query search (name: Option Text, after: Option Time) -> Set User =
  from u in users
  where match name  | None -> true | Some n -> u.name == n
  where match after | None -> true | Some t -> u.created > t
  select u
```

The planner specialises per pattern of present parameters, caching a
plan for each.

## lang/variants

```
query search (...) -> Set User =
  from u in users
  optional where u.name == name
  optional where u.created > after
  sort by one of { u.name, u.created }
```

Each combination is typed at registration; the client picks one by
flags.

## lang/adhoc-only

Run-time composition only for the ad hoc role.
