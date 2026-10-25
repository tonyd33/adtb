---
title: How a write changes a row's constructor
pick: one
raised-by: [union-index/retag/move]
options:
  clause:
    title: A retag clause naming the new constructor and its full payload
  assign:
    title: Assigning a constructor value to the row; retag is a different constructor
  carry:
    title: Fields with the same name and type carry over; the rest must be given
    assessment: {grade: hack, why: identity of a field decided by its spelling}
---
The row keeps its key and any sum-level fields, and moves heaps. Old-constructor fields are gone
with the old tuple; references from them vanish; incoming FKs to the
old constructor fail the retag (`fk/retag/recheck`). New-constructor
uniques and FKs are checked as for an insert.

## write/retag/clause

```
update i in identities.EmailPass where i.id == k
retag Google { gid: g }
```

Every `Google` field is supplied; omitted ones are a type error unless
defaulted. The old binder `i` is in scope in the payload.

## write/retag/assign

```
update i in identities where i.id == k
set match i
  | EmailPass e -> Google { gid: lookup(e.email) }
  | Google g    -> g
```

An update produces a whole constructor value per row; the engine moves
the row when the result's constructor differs. One form covers
in-constructor update, replace and retag.

## write/retag/carry

```
update i in identities.EmailPass where i.id == k retag Google { gid: g }
-- EmailPass.created_at and Google.created_at: carried
```

Fields of the old constructor whose name and type match a field of the
new one are copied.
