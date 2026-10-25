---
title: How a transaction that returns results is written
pick: some
raised-by: [write/surface/txdata]
options:
  block:
    title: A transact block computes a change set and a result from one snapshot
  registered-txn:
    title: Registered transaction functions, called as one round trip
    needs: [surface/registration/unit/bundle]
  interactive:
    title: A client transaction spans several registered calls, committed by the client
---
Queries are pure and a write is a change-set value, so "insert and
return the new key" needs a form that names the change set and reads
from it. The returning-rows ruling (`write/insert/returns/rows`) is the
result of that read.

## write/transact/block

```
transact
  let u  = insert users { name: n }
  let i  = insert identities.Google { user: key u, gid: g }
  return key i
```

Every `let` binds a change-set value; nothing is applied until the
block ends. `insert` yields the rows it would insert, so `key i` is the
generated key. The block's change set is one multiple assignment;
reads inside it see the snapshot, not the pending rows, except through
the bound names.

## write/transact/registered-txn

```
txn sign_up (n: Text, g: Text) -> Key identities = transact ...
```

A registered transaction is called by name like a query; one round
trip, retried whole on serialization failure.

## write/transact/interactive

```
let t = db.begin()
let u = t.call(find_user, email)
t.call(add_identity, u, g)
t.commit()
```

Several calls in one SSI transaction; the client retries the whole
closure.
