---
title: Which schema changes to a sum are supported
pick: any
options:
  add:
    title: Add a constructor
  remove:
    title: Remove a constructor
  split:
    title: Split a constructor into several
  merge:
    title: Merge constructors into one
  move-field:
    title: Move a field between constructors or up to the sum
  rename:
    title: Rename a constructor
  nest:
    title: Nest constructors under an intermediate sum, or un-nest
---
Operations on a sum's constructor set that the system migrates data and
queries through, beyond hand-written migrations.

## evolution/add

`add constructor Identity.Apple { sub: Text }`. Existing rows are unchanged.

## evolution/remove

Drop `Facebook`. Existing `Facebook` rows must be migrated or refused.

## evolution/split

`OAuth { provider: Google | Apple, sub: Text }` becomes
`Google { gid: Text } | Apple { sub: Text }`, each
row routed by a function of its fields.

## evolution/merge

`Google | Apple` becomes `OAuth { provider: Google | Apple, sub: Text }`.

## evolution/move-field

`created_at` present in every constructor moves to the sum, or the
reverse.

## evolution/rename

`Facebook` becomes `Meta`. Postgres has `ALTER TYPE ... RENAME VALUE`
for enums.

## evolution/nest

`Google | Apple` becomes `OAuth (Google | Apple)`. Rows keep their
leaf constructor; matches on the outer sum change shape.
