---
title: What stops when corruption is detected
pick: one
raised-by: [buffer/faults/corrupt-detect]
options:
  engine:
    title: The engine halts
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
  heap:
    title: The constructor heap or index holding the page is quarantined; queries not reading it continue
  call:
    title: The call that read the page fails; the next call tries again
---
Settled: corrupted pages are detected and the engine never returns
wrong rows. One heap per constructor makes a constructor the natural
blast radius; compile-time pruning (settled) knows which heaps a query
can read.

## ops/halt-scope/engine

TigerBeetle halts on a fault it cannot repair. Safe; one bad page of a
rarely used constructor stops everything.

## ops/halt-scope/heap

```
error heap_quarantined identities.Apple (page 88123 checksum)
-- queries over identities.(EmailPass | Google) run; a match over identities fails
```

Writes are refused where a constraint or FK check must read the
quarantined heap. A union-index page holds entries of several
constructors, so a bad index page quarantines the index, and with it
every plan using it.

## ops/halt-scope/call

Transient read errors recover; a persistent one fails every call that
touches it, with no state recording it.
