---
title: Whether a family name may read as a bundle name
pick: one
options:
  refuse:
    title: Refuse at registration a family whose name is itself FAMILY-VERSION
  allow:
    title: Allow it, and a family grant on it can never be written
---
`web-2-1.0` is version `1.0` of family `web-2`. `grant call web-2::q` reads as query `q` of bundle `web-2`, so that family cannot take a family grant.

## security/family-name-shape/refuse

Refused when the bundle is first registered, naming the family. A name means one thing, and the failure comes when the author can still rename, not when a grant is first tried.

## security/family-name-shape/allow

Leaves a family that silently cannot use a form every other family has.
