---
title: "How grants follow structural operators"
pick: one
options:
  carry:
    title: "Rename and nest carry; split grants each new constructor the kept fields; merge requires equal source grants; remove drops"
  drop:
    title: "Grants on changed constructors are dropped"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/grant-carry/carry

Rename and nest carry; split grants each new constructor the kept fields; merge requires equal source grants; remove drops. Schema changes must not silently revoke or widen access.

## spec/grant-carry/drop

Grants on changed constructors are dropped. Every split silently revokes access.
