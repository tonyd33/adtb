---
title: A change other than add breaks every live bundle at once
pick: any
raised-by: [surface/registration/order/pinned]
options:
  translate:
    title: Bundles keep running at their pinned version, translated through operator inverses
    needs: [engine/pinned-version]
    links:
      - {rel: prior-art, to: prior/prism-bidel}
  add-only-online:
    title: State that only add is online; every other operator needs affected bundles retired first
  live:
    title: Live with forced client upgrades for any non-add change
    excludes: [surface/version-serving/translate, surface/version-serving/add-only-online]
---
`rename constructor Identity.Facebook to Meta` makes every live bundle that names
`Facebook` fail its recheck, so `on-change/refuse` refuses the change,
and a bundle written for the new name cannot register before the
version exists. Only `add` can roll out with old bundles still live.

## surface/version-serving/translate

A bundle pinned to version 8 keeps running after the log reaches 9.
Its queries are rewritten through the inverse of each operator between
8 and 9, as InVerDa serves every schema version over shared data. A
rename is translated exactly; a split is translated through its
routing match; a constructor added after 8 is invisible at 8, the same
as a pinned old reader. Operators were made closed under inverses for
this.

## surface/version-serving/add-only-online

The manual says so: rename, split, merge, move-field and nest are
offline for the bundles they touch.

## surface/version-serving/live

No statement; clients upgrade in lockstep.
