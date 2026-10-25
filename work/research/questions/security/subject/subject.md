---
title: What a privilege is granted on
pick: some
options:
  relations:
    title: Grants on relation variables and constructor sets, checked against what a query reads
    links:
      - {rel: prior-art, to: prior/security-pg-grant}
  queries:
    title: Grants on registered queries and bundles; calling one is the privilege
    links:
      - {rel: prior-art, to: prior/surface-convex}
      - {rel: gains, to: surface/adhoc/scoped}
---
Application roles run only registered queries; the scoped ad hoc role
types query text. Either kind of grant can serve either role, and a
design may keep both: query grants for applications, relation grants for
ad hoc readers.

## security/subject/relations

```
grant read identities.(EmailPass | Google) to analyst
grant write invoices.Draft to clerk
```

A query is admitted when every constructor relation it reads or writes
is granted. Registered queries name the constructor relations they read
(`mvcc/long-reader/ctor-scoped`), so the check runs at registration and
again on each schema change.

## security/subject/queries

```
bundle web-2.3 pins 9 (a91c...)
query login_key (id: Uuid) -> Set (Key users) =
  from i in identities where i.id == id select i.user

grant call web-2.3 to web
grant call web-2.3::login_key to support
```

Stored-procedure-only access, close to a capability: the role holds the
right to run a fixed, typed, parameterised computation and nothing else.
Convex exposes only public functions; internal ones cannot be called by
clients.
