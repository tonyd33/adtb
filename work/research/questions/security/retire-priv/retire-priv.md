---
title: Who may retire a live bundle
pick: one
raised-by: [surface/reader-tracking/dormant/retire, surface/registration/on-change/retire]
options:
  owner-or-admin:
    title: The bundle's owner or an administrator
  schema-author:
    title: Any role allowed to append an operator the bundle blocks
---
Retiring a bundle stops every client built against it. A change that
breaks a live bundle is refused until the bundle is retired.

## security/retire-priv/owner-or-admin

The schema author asks the bundle's owner.

## security/retire-priv/schema-author

```
$ adt migrate 0011
refused: bundle mobile-1.9 reads identities.EmailPass.pw
$ adt retire mobile-1.9 && adt migrate 0011
```
