---
title: What a restore answers for a damaged backup part
pick: one
options:
  named-part:
    title: BackupDamaged names the part, checked before a byte of the new file is written
  dump-damaged:
    title: Reuse DumpDamaged with an offset
---
A backup is many parts; a dump is one stream.

## repl/backup-damaged/named-part

An operator replaces a part by name. No partial file is left.

## repl/backup-damaged/dump-damaged

An offset names no part of a directory.
