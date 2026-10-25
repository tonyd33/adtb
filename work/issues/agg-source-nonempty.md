---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 97
summary: "the kernel types the source of min, max and avg over a group as Set, where semantics requires Set⁺"
closed: 2026-10-25
tags:
  - lang
---

# min, max and avg over a group get a possibly empty source

## Report

Found by the reference interpreter. For `min`, `max` and `avg` over a
group, the kernel types the source as a plain `Set`. The semantics
requires `Set⁺`, and these aggregates are defined only on a nonempty
set.

Expected: the checker gives the source the nonempty type, and
`docs/design/kernel.md` states it.

## 2026-10-25: resolved

Fixed in `fix(lang): decimal literals, constructor joins, aggregate sources, anonymous sum indices and ad hoc checks`. The source of `min`, `max` and `avg` over a group is `Set⁺`.
