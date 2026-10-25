---
title: How a constructor set is named in keys, indexes and references
pick: some
raised-by: [index/ctor-set]
options:
  list:
    title: An explicit list of constructors
  named:
    title: Named subsets declared in the schema
  except:
    title: The sum minus listed constructors
---
Every index, unique constraint and FK target ranges over a set of
constructors of one sum. The sum's own name always denotes the whole
set. These forms stack.

## surface/ctor-set/list

```
relation identities : Identity
  key id
  unique (sub) in (Google | Apple)

owner: Key identities.(Google | Apple)
```

## surface/ctor-set/named

```
set OAuth of identities = Google | Apple
relation identities : Identity
  key id
  unique (sub) in OAuth

owner: Key identities.OAuth
```

A name for a set used in several places. Distinct from nesting: rows
and matches are unchanged.

## surface/ctor-set/except

```
relation identities : Identity
  key id
  index (created_at) in (except EmailPass)
```

Grows when a constructor is added.
