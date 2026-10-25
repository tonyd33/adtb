---
title: "How recovery finds the end of the log"
pick: one
options:
  batch-framing:
    title: "Blocks carry batch sequence and block count; incomplete batches discarded; the tail zeroed before new writes"
  crc-only:
    title: "Per-block checksums only"
---
Raised by the second review of the spec in docs/spec/.

## spec/log-framing/batch-framing

Blocks carry batch sequence and block count; incomplete batches discarded; the tail zeroed before new writes. Stale blocks of a discarded batch keep valid checksums and could be read as a continuation.

## spec/log-framing/crc-only

Per-block checksums only. A second crash can misread the tail.
