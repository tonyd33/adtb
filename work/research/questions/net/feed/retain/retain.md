---
title: A disconnected feed consumer holds back the log
pick: any
raised-by: [net/feed/changes]
options:
  window:
    title: Positions older than a retention window are refused; the consumer resnapshots
  checkpoint:
    title: The server keeps each named consumer's position, as a replication slot
    links:
      - {rel: prior-art, to: prior/net-pg-notify}
  live:
    title: Live with feeds retaining log without bound
    excludes: [net/feed/retain/window, net/feed/retain/checkpoint]
---
Postgres slots retain WAL until consumed and can fill the disk.

## net/feed/retain/window

```
subscribe changes identities from 0/0100   <- Error TooOld { oldest: 0/1000 }
```

## net/feed/retain/checkpoint

Named consumers; an operator drops a stuck one.

## net/feed/retain/live

Retained log grows until the consumer returns.
