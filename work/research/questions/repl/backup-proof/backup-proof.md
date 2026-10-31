---
title: Whether a backup runs its test restore, and where the result lives
pick: one
options:
  separate-proof:
    title: backup only copies; a separate proof step writes a proof file, and a backup without one is unproven
  always:
    title: backup always test restores and writes the result in its manifest
---
interop.md 7.1 says the tool records the result in the manifest.

## repl/backup-proof/separate-proof

A test restore takes double the disk and may need host functions a server does not have. The manifest stays immutable and its hash names what the proof proves.

## repl/backup-proof/always

The manifest is the last write of a backup and marks it complete; a result added later rewrites it.
