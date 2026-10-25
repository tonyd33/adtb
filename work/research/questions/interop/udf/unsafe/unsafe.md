---
title: A native plugin runs unsandboxed in the engine process
pick: any
raised-by: [interop/udf/native-plugin]
options:
  trusted-only:
    title: Only the operator of the deployment can install plugins
  out-of-process:
    title: Plugins run in a separate process over IPC
  live:
    title: Live with plugin faults corrupting pages
    excludes: [interop/udf/unsafe/trusted-only, interop/udf/unsafe/out-of-process]
---
A plugin can write into the buffer pool. Postgres marks `trusted`
extensions "extremely exposed" to schema-based attacks
(https://www.postgresql.org/docs/current/extend-extensions.html).

## interop/udf/unsafe/trusted-only

Installation is a privilege, not a schema operator any role may run.

## interop/udf/unsafe/out-of-process

The plugin crashes alone; every call pays a context switch.

## interop/udf/unsafe/live

Corruption is detected by page checksums (`layout/checksum/in-page-32`)
after the fact.
