---
title: "Declaration keywords for types and relation variables"
pick: one
options:
  type-relation:
    title: "type and relation"
  data-var:
    title: "data and var"
---
One form declares types, another relation variables; a record is a one-constructor type.

## syntax/decl/type-relation

```
type Identity = EmailPass { email: Text, hash: Bytes } | Google { gid: Text }
type User = { name: Text }
relation identities : Identity key id
```

## syntax/decl/data-var

```
data Identity = ...
var identities : Identity key id
```
