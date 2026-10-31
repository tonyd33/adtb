---
title: What a backup or restore target names at beta
pick: one
options:
  paths:
    title: File system paths only; a URL scheme is refused by name
  object-store:
    title: Paths and s3 URLs
---
The spec's examples use `s3://`. There is no object store dependency.

## repl/backup-target/paths

An object store mounts or syncs from a directory. The audited allowlist stays short.

## repl/backup-target/object-store

Adds a large dependency with its own threads and clock for what a sync tool does.
