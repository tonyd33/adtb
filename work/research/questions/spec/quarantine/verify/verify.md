---
title: "Whether a verify finding quarantines an index"
pick: one
raised-by: [ops/halt-scope/heap]
options:
  yes:
    title: "An Order, Orphan or Missing finding quarantines the index it names, as a bad page does"
  no:
    title: "Only a page that fails its checksum or log position quarantines; a finding only reports"
---
Raised by the milestone 9 audit. Catalog §11.2 names checksum and log
position. `verify` can find an index wrong with every page of it
intact: `verify` finds an orphan entry in `index (user)`.

## spec/quarantine/verify/yes

The index is known wrong, and the engine never returns wrong rows, so
queries stop using it and writes to its constructors are refused until
`reindex`. A bug in `verify` then costs a refused write and a reindex,
never a wrong answer.

## spec/quarantine/verify/no

Queries keep using an index the engine has just reported wrong and
return wrong rows until an operator reads the finding.
