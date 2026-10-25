---
title: How load beyond capacity is refused
pick: some
raised-by: [engine/embedded-server]
options:
  connections:
    title: A cap on connections and streams
  queue:
    title: Calls wait in a bounded queue; beyond it, Overloaded with a retry hint
  per-bundle:
    title: Quotas per bundle and per principal
    needs: [surface/registration/unit/bundle]
  classes:
    title: Priority classes, ad hoc below registered
    needs: [surface/adhoc/scoped]
---
TigerBeetle bounds sessions (`clients_max`, 64) and one in-flight
request each, so its queue size is static; Postgres bounds only
`max_connections`.

## net/admission/connections

QUIC MAX_STREAMS, per-connection stream limits.

## net/admission/queue

```
<- 7 Error Overloaded { retry_after: 50ms }
```

## net/admission/per-bundle

```
bundle web-2.4 pins 9 (a91c...) limits { concurrent: 200, rate: 5000/s }
```

## net/admission/classes

An analyst's scan cannot starve the application.
