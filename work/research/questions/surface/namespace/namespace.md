---
title: How schema names are namespaced
pick: one
options:
  flat:
    title: One namespace per database
  modules:
    title: Named modules with qualified references
    links:
      - {rel: prior-art, to: prior/gel}
---
Constructors are scoped by their sum (`Identity.Google`); the question
is the level above.

## surface/namespace/flat

```
type Identity = ...
relation identities : Identity key id
```

## surface/namespace/modules

```
module auth
type Identity = ...
relation identities : Identity key id
```

```
module billing
type Account = Owned { who: Key auth::identities } | ...
relation accounts : Account key id
```

Gel modules (https://docs.geldata.com/reference/datamodel/modules,
unverified URL). A module is also a natural unit for registered query
bundles and permissions.
