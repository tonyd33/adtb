---
title: A base write removes a derived row that a reference points at
pick: any
raised-by: [derived/mv-integrity/fk-target]
options:
  restrict:
    title: Refuse the base write, naming the referrer
    links:
      - {rel: same-as, to: write/delete-action/restrict}
  propagate:
    title: The referrer's declared delete action applies to the derived row's removal
    links:
      - {rel: same-as, to: write/delete-action/cascade}
  live:
    title: Live with references left dangling when the derived row goes
    excludes: [derived/mv-integrity/fk-delete/restrict, derived/mv-integrity/fk-delete/propagate]
    assessment: {grade: broken, why: the engine claims an FK it does not enforce}
---
Closing account 7 removes it from `active_accounts`; `flags` refers to
it. No one deleted from `active_accounts`, so a delete action has no
statement to attach to.

## derived/mv-integrity/fk-delete/restrict

The close is refused while `flags` refers to account 7.

## derived/mv-integrity/fk-delete/propagate

```
relation flags : { account: Key active_accounts on delete cascade, ... }
```

The view's computed deletion feeds the same delete-action machinery
as a direct delete, in the same multiple assignment.

## derived/mv-integrity/fk-delete/live

The base write commits and `flags` names an account no longer active.
