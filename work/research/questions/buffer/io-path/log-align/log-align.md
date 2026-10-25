---
title: Direct log writes must be whole aligned blocks
pick: any
raised-by: [buffer/io-path/direct]
options:
  tail-rewrite:
    title: Each flush rewrites the partial tail block, padded
  slot-pad:
    title: Each commit batch takes whole blocks, padding the rest
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
  live:
    title: Live with it
    excludes: [buffer/io-path/log-align/tail-rewrite, buffer/io-path/log-align/slot-pad]
---
`O_DIRECT` needs buffer address, length and offset aligned to the
logical block (usually 512 or 4096; `statx STATX_DIOALIGN` since 6.1).
A 200-byte commit record cannot be written alone.

## buffer/io-path/log-align/tail-rewrite

```
log block (4 KiB)
  0   u64  lsn of first record start in block
  8   u32  crc32c over 12..4095
  12  u16  bytes used
  14  ...  records, zero padding
```
Commit n rewrites block k; a torn rewrite can destroy commit n-1
already acknowledged in the same block. Needs two alternating tail
copies or per-record checksums so the old prefix survives.

## buffer/io-path/log-align/slot-pad

TigerBeetle pads each prepare to the maximum message size in a ring of
slots; a block once acknowledged is never rewritten. Costs space at
low batch sizes; group commit fills slots.
https://github.com/tigerbeetle/tigerbeetle/blob/main/src/vsr/journal.zig

## buffer/io-path/log-align/live

Not an option with `O_DIRECT`: misaligned writes fail with `EINVAL`
or silently fall back to buffered.
