---
title: Limits a bundle carries
pick: any
raised-by: [surface/registration/unit/bundle]
options:
  priority:
    title: A bundle declares its priority class for admission
    needs: [ops/admission/queue]
  rate:
    title: A bundle has a call rate limit enforced by the engine
  budget:
    title: A bundle has a share of query memory and CPU time
---
Bundles are the unit of registration and of version pinning
(proposed). They are also the natural tenant: a batch bundle and a web
bundle against one database.

```
bundle batch-1.2 pins 41 (5d2e...) priority background budget { memory: 25%, cpu: 2 cores }
```

## ops/quota/priority

Background bundles yield to regular ones in admission queues.

## ops/quota/rate

Calls per second per bundle; the protocol returns a retry-after.

## ops/quota/budget

Share of the per-query memory pool and of morsel workers.
