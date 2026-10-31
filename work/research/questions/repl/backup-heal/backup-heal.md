---
title: How a quarantined constructor relation comes back on an embedded or single file
pick: one
options:
  new-file:
    title: Restore a backup to a new file and swap it in with the database closed
  in-place:
    title: A command copies the relation's pages into the live file from a backup and the log
---
catalog.md 11.2 says an operator restores its pages. No command does, and `restore` writes a new file.

## repl/backup-heal/new-file

The one restore path heals. Commits after the backup's end are lost until an archive can roll forward.

## repl/backup-heal/in-place

A page copy path nothing else needs, and pages from the past must then be reconciled with later commits on other relations.
