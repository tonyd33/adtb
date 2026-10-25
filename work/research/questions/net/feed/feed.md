---
title: How clients learn of committed changes
pick: some
raised-by: [write/primitive/delta]
options:
  notify:
    title: Named channels with a payload, sent in a transaction and delivered at commit
    links:
      - {rel: prior-art, to: prior/net-pg-notify}
  changes:
    title: A feed of committed change sets per constructor, resumable from a log position
    links:
      - {rel: prior-art, to: prior/net-pg-notify}
      - {rel: prior-art, to: prior/net-cockroach}
  live:
    title: Subscriptions to registered queries, sent as inserted and deleted result rows
    links:
      - {rel: prior-art, to: prior/net-spacetimedb-sync}
      - {rel: prior-art, to: prior/net-convex-sync}
---
A write is a change set: inserted and deleted relations per
constructor (`write/primitive/delta`). That set is already the unit a
feed would ship.

## net/feed/notify

```
transact
  let i = insert identities.Google { ... }
  notify "identities" (key i)
```

Postgres LISTEN/NOTIFY: 8000-byte payload, folded within a
transaction, commit order across them.

## net/feed/changes

```
subscribe changes identities.(Google | EmailPass) from 0/1A2B
<- Push Changes { at: 0/1A2C, identities.Google: +[...] -[...] }
```

Postgres logical decoding per slot; CockroachDB changefeeds at least
once, ordered per key. A retag shows as a delete from one constructor
and an insert into another in the same change set.

## net/feed/live

```
query my_identities (u: Key users) -> Set Identity = from i in identities where i.user == u select i
subscribe my_identities(u)
<- Push Live { sub: 4, at: 0/1A2C, insert: [...], delete: [...] }
```

SpacetimeDB sends per-query deletes and inserts per committed reducer;
Convex reruns a query when a commit overlaps its read set and keeps a
client's query set at one timestamp.
