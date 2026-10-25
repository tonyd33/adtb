---
created-at: 2026-10-25
status: in-progress
kind: bug
priority: 98
summary: "an anonymous sum numbers its constructors in written order, so one type spelled two ways can compare its values unequal"
branch: lang
tags:
  - lang
---

# Anonymous sum constructor indices follow written order

## Report

Found by the reference interpreter. An anonymous sum's constructor
indices follow the order the constructors are written in. Two
spellings in different orders are one type, so a value of it built
under one spelling can compare unequal to the same value built under
the other.

Ruling: the constructor indices of an anonymous sum are in name order.
The ad hoc numbering on the wire is separate; `docs/spec/wire.md` §4.3
keeps it.
