---
title: How the server sends unrequested messages
pick: one
raised-by: [surface/wire/binary]
options:
  frames:
    title: Typed push frames on a reserved stream, at any time
    links:
      - {rel: prior-art, to: prior/net-resp3}
  between:
    title: Pushes only between calls, never inside a transaction
    links:
      - {rel: prior-art, to: prior/net-pg-protocol}
  none:
    title: No pushes; clients poll
---
Notices (bundle lease near expiry, shutdown, a constructor enabled),
notifications and feed updates all need a path.

## net/push/frames

```
<- 0 Push LeaseExpiring { bundle: ios-4.2, at: 2026-12-01 }
<- 0 Push Feed { sub: 4, changes: ... }
```

RESP3 `>` frames dispatched by kind.

## net/push/between

Postgres NotificationResponse arrives only outside a transaction,
possibly before ReadyForQuery.

## net/push/none

Notices go into the next reply; feeds are polled.
