---
title: Interactive transactions hold serializable state across round trips
pick: any
raised-by: [write/transact/interactive]
options:
  time-limit:
    title: A transaction open past a limit is aborted
    links:
      - {rel: prior-art, to: prior/net-fdb-client}
  promise:
    title: A call may take an earlier call's unreturned result as an argument
    links:
      - {rel: prior-art, to: prior/net-capnp-rpc}
  local:
    title: Offer interactive transactions only in process
    needs: [surface/wire/embedded]
  live:
    title: Live with longer conflict windows over the network
    excludes: [net/interactive/time-limit, net/interactive/promise, net/interactive/local]
---
Under SSI a transaction's reads stay tracked until commit; each round
trip widens the window in which a concurrent write makes it abort,
and the aborted work is redone from the client.

## net/interactive/time-limit

FoundationDB `transaction_too_old` at 5 s.

## net/interactive/promise

```
-> 1 Begin
-> 1 Call find_user (email)            as $u
-> 1 Call add_identity ($u.key, g)     -- sent at once
-> 1 Commit
```

One round trip for a dependent chain; the client still sees each
result. Cap'n Proto pipelining.

## net/interactive/local

Embedded callers pay no round trip; the server offers
`write/transact/registered-txn` and blocks.

## net/interactive/live

Clients retry whole closures (`engine/retry/retry-block`) more often.
