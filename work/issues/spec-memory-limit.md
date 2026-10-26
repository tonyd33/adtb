---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 298
summary: no StatementLimit reason names a memory limit; the only one carries a Duration
branch: adtb-spec3
tags:
  - spec
---

# Bug: No reason for a memory limit

## Report

`docs/spec/catalog.md` §8.1 says a call past its grant spills, and the
grant never grows. When it cannot spill (a hash build larger than
`query_memory`, with no pool pages or spill space to spare), the
statement is rejected, but `StatementLimit { limit: Duration }` names
only a time limit.

My ruling: `StatementLimit` carries `limit: Limit`, with
`Limit = Time { after: Duration } | Memory { bytes: I64 } |
Rounds { rounds: I64 }`. The last is the fixpoint round limit. State
it in catalog.md §4 and semantics.md §5.10, and update every example.
