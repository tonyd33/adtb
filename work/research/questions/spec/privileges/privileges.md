---
title: "The privilege set"
pick: one
options:
  fifteen:
    title: "read, insert, update, delete, retag, call, adhoc, assert, schema, monitor, cancel, maintain, configure, read_files, write_files"
  per-file:
    title: "Each part lists its own"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/privileges/fifteen

read, insert, update, delete, retag, call, adhoc, assert, schema, monitor, cancel, maintain, configure, read_files, write_files. One list in syntax and security; the key field is granted with any read.

## spec/privileges/per-file

Each part lists its own. The lists disagreed.
