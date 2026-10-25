---
title: "How a server releases an old bundle for a new release"
pick: one
options:
  command:
    title: "`supersede bundle B by B2`, run by B's owner or the schema owner, B2 live in the same family"
  retire-only:
    title: "Only retirement or expiry release a bundle"
---
Raised by the third review of the spec in docs/spec/.

## spec/supersede/command

`supersede bundle B by B2`, run by B's owner or the schema owner, B2 live in the same family. A new release on a server otherwise leaves the old one live until retirement or expiry.

## spec/supersede/retire-only

Only retirement or expiry release a bundle. A migration stays blocked for a whole lease.
