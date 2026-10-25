---
title: "What an upsert's else branch sees"
pick: one
options:
  old-and-new:
    title: "Both old and the proposed row new; a single-row let binds exactly one row"
  old-only:
    title: "Only old"
---
Raised by the second review of the spec in docs/spec/.

## spec/upsert-new/old-and-new

Both old and the proposed row new; a single-row let binds exactly one row. A bulk merge must read the incoming row.

## spec/upsert-new/old-only

Only old. Bulk merges impossible.
