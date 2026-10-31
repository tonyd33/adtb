---
title: What a backup through a server needs beyond maintain
pick: one
options:
  write-files:
    title: maintain and write_files, confined as dump is
  maintain:
    title: maintain alone
---
catalog.md 9.1 asks only `maintain`, which would let a maintainer write any path the server can.

## repl/backup-files/write-files

Same rule as dump and archive audit.

## repl/backup-files/maintain

A write to any server path, and an escape from the export roots.
