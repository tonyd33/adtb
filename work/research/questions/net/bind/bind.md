---
title: Where a client states its bundle and pinned schema version
pick: one
raised-by: [surface/registration/unit/bundle, surface/registration/order/pinned]
options:
  connection:
    title: The handshake binds the connection to one bundle at its version
  call:
    title: Each call names its bundle
  none:
    title: Calls name query hashes only; the engine finds the bundle from the hash
    assessment: {grade: hack, why: a query shared by two bundles leaves the pinned version ambiguous}
---
A bundle pins a schema version; the engine encodes results and checks
calls against it, and the bundle's lease is renewed by connecting
clients (`surface/reader-tracking/dormant/lease`).

## net/bind/connection

```
-> Hello { bundle: web-2.4, version: 9 (a91c...), principal: ... }
<- HelloOk { session: 77 }
<- Error Retired { bundle: web-2.3 }        -- for a retired bundle
```

Retirement and lease renewal happen once, at connect. A pooled
connection serves one bundle.

## net/bind/call

```
-> Call { bundle: web-2.4, query: login_key, args: [...] }
```

One connection serves several bundles, as a shared proxy would.

## net/bind/none

Apollo persisted query hashes.
