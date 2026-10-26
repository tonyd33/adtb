---
created-at: 2026-10-25
status: open
kind: feature
priority: 741
summary: "aggregate constraints recompute each touched group by index probe; nothing maintains aggregates or applies commutative increments"
tags: [engine]
---

# Feature: maintained aggregates and commutative increments

## Report

`docs/design/maintenance.md` sections 4 and 5. A count, sum, min or max constraint is checked by probing its whole group, so the cost is the group's size, not the change's.
