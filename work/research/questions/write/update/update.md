---
title: How an update addresses fields of a constructor
pick: some
raised-by: [sum/relation]
options:
  ctor-binder:
    title: The update binds a constructor relation and sets its fields
  match-set:
    title: The update binds the sum and sets fields per arm of a match
---
Fields belong to constructors; only the key and sum-level fields are
common. An update must say which constructor's fields it sets. An arm
yielding another constructor retags the row (`write/retag/assign`).

## write/update/ctor-binder

```
update g in identities.Google where g.user == u set Google { g with gid: "g-456" }
```

Update-by-query: the binder ranges over a constructor relation filtered
by any predicate, as `from` does.

## write/update/match-set

```
update i in identities where i.user == u
set match i
  | EmailPass e -> EmailPass { e with hash: rehash(e.hash) }
  | Google g    -> g
```

Exhaustive as any match is; each arm gives the row's new whole value,
and `Google g -> g` leaves it unchanged.
