---
title: What export does to an existing file
pick: one
options:
  refuse:
    title: Refused
  overwrite:
    title: Replace it
---
The spec is silent; backup and restore refuse an occupied target.

## security/export-occupied/refuse

A role with write_files must not destroy another's export.

## security/export-occupied/overwrite

Replaces any file in the roots.
