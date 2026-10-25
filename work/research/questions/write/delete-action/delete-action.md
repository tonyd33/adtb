---
title: What deleting a referenced row does to its referrers
pick: some
raised-by: [fk/to-sum, fk/to-ctor, fk/to-subset]
options:
  restrict:
    title: Refuse the delete while referrers exist
  deferred-restrict:
    title: Refuse at commit if referrers still exist
    links:
      - {rel: prior-art, to: prior/write-edgeql}
  cascade:
    title: Delete the referrers
    links:
      - {rel: prior-art, to: prior/write-datomic-tx}
  retag-referrer:
    title: Retag the referrer to a declared constructor without the reference
  set-member:
    title: Set a value-level sum field holding the reference to a payload-free constructor
    needs: [model/refs-anywhere]
---
There is no NULL, so SQL's `SET NULL` has no meaning; its job falls to
a constructor that carries no reference. Retag of the target is
governed by `fk/retag`.

## write/delete-action/restrict

```
type Session = { identity: Key identities on delete restrict }
```

Gel's default for links.

## write/delete-action/deferred-restrict

Gel `on target delete deferred restrict`: the error waits for commit,
so a transaction may delete and re-create the target or re-point the
referrer.

## write/delete-action/cascade

```
type Reset = { identity: Key identities.EmailPass on delete cascade }
```

Gel `delete source`; Datomic `:db/retractEntity` retracts datoms that
reference the entity.

## write/delete-action/retag-referrer

```
type Session = Active { identity: Key identities on delete retag Revoked { at: now() } }
             | Revoked { at: Time }
```

The referrer moves to a constructor without the field, the relation
level `SET NULL`. The new payload is given in the declaration.

## write/delete-action/set-member

```
type Device = { owner: None | Owned { user: Key users on delete set None } }
```

`SET DEFAULT` with the default a constructor of the field's sum. The
reference sits inside a value-level constructor, which `sum/value`
otherwise rules out.
