---
title: "What a damaged page outside any user relation quarantines"
pick: one
raised-by: [ops/halt-scope/heap]
options:
  database:
    title: "The database: every call fails with Quarantined, and it opens for verify and dump only"
  own-object:
    title: "Each engine structure has its own mark, and calls that need it fail"
---
Raised by the milestone 9 audit. A commit log page fails, or a catalog,
allocation or marks page does. No constructor relation holds it. No
user `repair` command exists, and none is planned: the library pass that frees leaked pages stays internal.

## spec/quarantine/unowned/database

Every statement reads the catalog or the commit log, so a smaller mark
would stop the same calls with more states to test. `verify` and `dump`
still save what can be read. An engine relation with its own heap, such
as `sys::verify_findings`, is a relation like another and takes the
constructor mark.

## spec/quarantine/unowned/own-object

Finer marks for structures almost every call needs: more states, the
same calls stopped.
