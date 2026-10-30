---
title: "Which schema-owner role a restore keeps"
pick: one
options:
  file:
    title: "The file's owner role; the dump's owner row maps to it, and a dump naming the owner differently is refused"
  dump:
    title: "The dump's owner row replaces the file's"
---
Raised by the spec-rev30 review (G19).

## spec/restore-owner/file

A restore keeps the file's schema-owner role. The dump's `owner` row maps to it,
and a dump whose security section names the owner differently is refused.

## spec/restore-owner/dump

The dump's row replaces the file's, which can rename the role the engine relies on.
