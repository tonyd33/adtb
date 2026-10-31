---
title: Whether a quarantined source can be backed up
pick: one
options:
  heap-yes-file-no:
    title: A quarantined relation or index is copied and listed in the manifest, and its proof fails; a quarantined database is refused
  refuse-all:
    title: Refuse any quarantined source
---
The moment of damage is when a copy is most wanted.

## repl/backup-quarantined/heap-yes-file-no

A partial copy beats none, and the manifest says what is damaged. Catalog 11.2 allows only verify and dump on a quarantined database.

## repl/backup-quarantined/refuse-all

Loses the intact relations' only fresh copy.
