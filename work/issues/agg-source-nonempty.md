---
created-at: 2026-10-25
status: open
kind: bug
priority: 97
summary: "the kernel types the source of min, max and avg over a group as Set, where semantics requires Set⁺"
tags: [lang]
---

# min, max and avg over a group get a possibly empty source

## Report

Found by the reference interpreter. For `min`, `max` and `avg` over a
group, the kernel types the source as a plain `Set`. The semantics
requires `Set⁺`, and these aggregates are defined only on a nonempty
set.

Expected: the checker gives the source the nonempty type, and
`docs/design/kernel.md` states it.
