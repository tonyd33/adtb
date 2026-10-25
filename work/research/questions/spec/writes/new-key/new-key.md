---
title: "When a key change in an update is refused"
pick: one
options:
  ctor-expr:
    title: "A constructor expression giving a key field is a type error; KeyChanged is the dynamic failure for every other new value"
  checker-visible:
    title: "A type error where the type checker can see the key differs, KeyChanged otherwise"
---
Raised by the implementability review of docs/spec/. writes §3.3 made
a key change a type error "where the type checker can see" it, so
acceptance depended on the checker.

## spec/writes/new-key/ctor-expr

An update's new value written as a constructor expression may not give
a key field: a type error. `KeyChanged` is the dynamic failure for every
other new value whose key differs.

```adt
update g in identities.Google where g.user == u
set Google { g with id: k }   -- type error
```

## spec/writes/new-key/checker-visible

A type error where the checker can see that the key differs. Which
programs are accepted is relative to how clever the checker is.
