---
title: "Whether scripts wait for pinned bundles"
pick: one
options:
  never:
    title: "Never; a script changes data, so translation across it is the identity"
  block:
    title: "Refused while any live bundle pins an earlier version"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/scripts-pinning/never

Never; a script changes data, so translation across it is the identity. Blocking would stop every data-fix migration for a lease.

## spec/scripts-pinning/block

Refused while any live bundle pins an earlier version. Blocks data fixes.
