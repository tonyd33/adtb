---
title: The storage fault model durability is promised under
pick: one
raised-by: [substrate/new-engine]
options:
  crash-only:
    title: Crash stop only; synced writes survive, unsynced writes may vanish
  torn:
    title: Crash stop plus torn writes of any multi-sector write
  corrupt-detect:
    title: Also bit rot, latent sector errors and misdirected writes, detected and halted on
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
  repair:
    title: Also detected faults repaired from redundant copies
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
---
What the spec says survives, and what the simulator injects. Each
model adds faults to the previous. TigerBeetle's safety doc cites
silent corruption in 0.031% of SSDs per year (Maneas et al. FAST
2020) and 1.4% of enterprise HDDs (Bairavasundaram et al. FAST 2008),
misdirected I/O in 0.023% of SSDs and 0.466% of nearline HDDs.
https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/concepts/safety.md

## buffer/faults/crash-only

The classic crash-consistency model. Postgres before checksums. Says
nothing about a sector that reads back wrong.

## buffer/faults/torn

Adds partial page writes: a 4 KiB write on a device with 512-byte
physical sectors is 8 writes, any subset of which may land. Answered
by `layout/torn`.

## buffer/faults/corrupt-detect

Every page and log block checksummed with its own address mixed in
(`layout/checksum`); a mismatch stops the engine rather than return
wrong rows.

```
on read(pid): verify crc(page) and page.pid == pid, else halt(pid)
```

## buffer/faults/repair

TigerBeetle: immutable, checksummed, hash-chained blocks; a bad block
is refetched from a replica; halts only when every copy is bad. A
single node needs local redundancy (mirrored log, multiple superblock
copies) for the log and roots at least.
