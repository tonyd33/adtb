---
title: Merging constructors loses the tag and re-scopes uniqueness
pick: any
raised-by: [evolution/merge]
options:
  discriminator:
    title: Merged constructor keeps the old tag as a field
  live:
    title: Live with the merge losing information
    excludes: [evolution/merge-tag/discriminator]
---
Merging `Google { gid: Text } | Apple { sub: Text }`, with `unique (gid)
in Google` and `unique (sub) in Apple`, into `OAuth { sub: Text }`
forgets which provider a row came from, and `gid` and `sub` were unique
separately. PRISM flags `MERGE TABLE` as not information preserving.

## evolution/merge-tag/discriminator

`OAuth { provider: Google | Apple, sub: Text }` with
`unique (provider, sub) in OAuth`.
The merge is then the exact inverse of the split (unverified as a
general claim).

## evolution/merge-tag/live

Old tag dropped; uniqueness widened or dropped.
