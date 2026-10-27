---
created-at: 2026-10-27
status: open
kind: bug
priority: 799
summary: "a text key longer than about 1600 bytes fails with fatal TooLong instead of a rejection"
tags:
  - engine
---

# Bug: an oversized key is a fatal error

## Report

Inserting a row whose text key is about 20000 bytes fails with `fatal: storage failure: TooLong { len: 20013, max: 1618 }`. An oversized index key is the user's input, not a storage failure: reject the statement with a reason naming the key and its limit, and state the limit in the spec.
