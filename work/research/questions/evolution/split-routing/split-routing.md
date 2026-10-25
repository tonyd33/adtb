---
title: Splitting needs a total single-valued routing of rows
pick: any
raised-by: [evolution/split]
options:
  match-route:
    title: Route by a match on the old fields, checked exhaustive
  catch-all:
    title: Unrouted rows go to an added catch-all constructor
  reject:
    title: Fail the migration on any unrouted row
  live:
    title: Live with BiDEL-style auxiliary tables for unrouted rows
    excludes: [evolution/split-routing/match-route, evolution/split-routing/catch-all, evolution/split-routing/reject]
links:
  - {rel: prior-art, to: prior/prism-bidel}
---
Constructors are disjoint and exhaustive, so every old row must go to
exactly one new constructor. BiDEL `SPLIT` allows overlapping conditions
(twins) and uncovered rows (sent to an auxiliary table). Routing on data
outside the row is beyond lenses.

## evolution/split-routing/match-route

```
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: sub }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub }
```

The routing match goes through the ordinary exhaustiveness checker.

## evolution/split-routing/catch-all

A final arm `| OAuth o -> OtherOAuth { sub: o.sub }`, with `OtherOAuth`
added to the target sum.

## evolution/split-routing/reject

The migration aborts listing unrouted rows.

## evolution/split-routing/live

Unrouted rows kept outside the sum.
