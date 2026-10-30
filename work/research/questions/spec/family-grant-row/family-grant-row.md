---
title: "How many rows a family call grant holds, and what a regrant does"
pick: one
options:
  latest:
    title: "One row per grantee, family and query with the latest signature; a regrant replaces it"
  per-signature:
    title: "One row per recorded signature; a regrant adds a row"
---
Raised by the spec-rev30 review (G9). A regrant can record S2 after `web-1` (S1) is
superseded and `web-2` (S2) registered. The spec does not say whether that is one
row or two, whether `web-1` is covered once live again, or what `revoke call
web::q` matches.

## spec/family-grant-row/latest

One row per grantee, family and query holds the latest recorded signature. A
regrant replaces it. `revoke call web::q` deletes that row. A bundle that is live
again with an older signature is not covered. The row's target text carries the
signature.

## spec/family-grant-row/per-signature

Each regrant adds a row for its signature, so every signature ever granted stays
covered until revoked one by one.
