---
title: Rules writing the same row can give order-dependent results
pick: any
raised-by: [derived/rules/change-rules]
options:
  simultaneous:
    title: Every rule of a round sees the same input; conflicting outputs are refused
    links:
      - {rel: prior-art, to: prior/write-datomic-tx}
  priority:
    title: Rules declare an order and later rules see earlier ones' output
  live:
    title: Live with an unspecified firing order
    excludes: [derived/rules/confluence/simultaneous, derived/rules/confluence/priority]
    assessment: {grade: broken, why: the committed state depends on an order the user cannot see}
---
Two rules on one change both update `accounts[7].flag`.

## derived/rules/confluence/simultaneous

A round's outputs are unioned into one change set; two different new
values for one key refuse the transaction, as Datomic refuses
conflicting datoms (`:db.error/datoms-conflict`).

## derived/rules/confluence/priority

Postgres fires same-event triggers alphabetically by name.

## derived/rules/confluence/live

Implementation order decides.
