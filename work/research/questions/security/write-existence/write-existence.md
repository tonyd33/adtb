---
title: Whether a write may tell a referenced row exists
pick: one
options:
  stated:
    title: A role with a write grant can tell by the constraint's failure, and the spec says so
  hide:
    title: Refuse a reference to a relation the role cannot read
---
`insert links { to: secrets.key(1) }` by a role with no read on `secrets` succeeds, and `key(99)` fails naming the constraint.

## security/write-existence/stated

The write grant was given for that relation and its constraints hold for every writer. Same class as a uniqueness refusal.

## security/write-existence/hide

Makes a foreign key unwritable without read of its target's rows.
