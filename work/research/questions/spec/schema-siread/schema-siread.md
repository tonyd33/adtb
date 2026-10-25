---
title: "How writers at an older version serialize against a migration"
pick: one
options:
  version-item:
    title: "Every statement reads the schema version item; a migration writes it"
  none:
    title: "No dependency"
---
Raised by the second review of the spec in docs/spec/.

## spec/schema-siread/version-item

Every statement reads the schema version item; a migration writes it. A writer at n-1 could commit into a constructor version n deprecated without any conflict.

## spec/schema-siread/none

No dependency. Not serializable.
