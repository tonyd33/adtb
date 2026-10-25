---
title: Store limits cap the size of one value
pick: any
raised-by: [[layout/kv-row/row-per-key, engine/fdb]]
options:
  split:
    title: Rows over the limit are split across consecutive suffixed keys
    links:
      - {rel: prior-art, to: prior/engine-fdb-record-layer}
  live:
    title: Refuse rows over the limit
    excludes: [layout/kv-row/split/split]
---
FoundationDB rejects values over 100 kB and transactions over 10 MB.
https://apple.github.io/foundationdb/developer-guide.html

## layout/kv-row/split/split

```
(identities, Google, 7, 0) -> bytes 0..100000
(identities, Google, 7, 1) -> bytes 100000..
```

Record Layer splits records above about 100 KB; a read is a range read
of the row's suffixes.

## layout/kv-row/split/live

Rows stay under 100 kB; large values go elsewhere.
