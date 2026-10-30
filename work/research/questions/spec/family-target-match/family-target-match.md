---
title: "How a family target with a signature matches a row"
pick: one
options:
  ids-at-v:
    title: "By the ids its names have at its own version; the row keeps the version recorded when granted"
  text:
    title: "By its printed text"
---
Raised by the spec-rev30 review round 3 (N9). A family row's version is fixed when
granted. A revoke or grant written at a later version with the same signature prints
a different text.

## spec/family-target-match/ids-at-v

A family target that carries a signature matches by the ids its names have at its
own `v`, not by text. The row prints the `v` recorded when it was granted, and a
regrant with equal ids keeps the row and its `v`.

## spec/family-target-match/text

A revoke must spell the row's own `v`, which the author cannot know without reading
the catalog.
