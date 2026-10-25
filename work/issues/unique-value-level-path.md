---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 93
summary: "the checker refuses unique (status.Paid.ref) with \".C selects constructors of a set\", though constraints.md §2.1 admits it"
closed: 2026-10-25
tags:
  - lang
---

# Bug: unique over a value-level constructor path is refused

## Report

The checker refuses `unique (status.Paid.ref)` with "`.C` selects
constructors of a set". `docs/spec/constraints.md` §2.1 admits an index
field that goes through a constructor of a value-level sum field.
`docs/design/kernel.md` must also state how such an index field says
which rows take part: only rows whose `status` is `Paid`.

Expected: the declaration checks, and its kernel form names the
constructor path.

## 2026-10-25: resolved

Fixed in `fix(lang): keep key draws unduplicated and index through value-level constructors`. An index field is a value plus a constructor path (`kernel.md` §1), and only rows holding the path take part.
