---
title: Error handling inside the engine
pick: one
raised-by: [substrate/new-engine]
options:
  layered-enums:
    title: An error enum per layer, converted at each boundary
  one-enum:
    title: One engine error enum shared by every layer
  fatal-split:
    title: Operating errors as Result; invariant violations crash and recover from the log
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
      - {rel: gains, to: buffer/fsync-fail/panic-replay}
---
TIGER_STYLE: assertions detect programmer errors and the only correct
handling is to crash; operating errors must all be handled (citing
Yuan et al., OSDI 2014: 92% of catastrophic failures came from mishandled
non-fatal errors). Settled: after an fsync failure the engine crashes
and replays.

## project/arch/errors/layered-enums

```
enum StorageError { Io(IoError), Corrupt { page: Pid } }
enum TxnError { Serialization, Storage(StorageError) }
```

## project/arch/errors/one-enum

`std::io::Error` shape: one type with a kind; any layer returns any
kind.

## project/arch/errors/fatal-split

```
fn read_page(&mut self, pid: Pid) -> Result<PageRef, IoError>  // disk said no
assert!(page.lsn() <= self.flushed_lsn);                         // engine is wrong: crash
```

Corruption detected by checksum (`buffer/faults/corrupt-detect`) is an
operating error; a broken invariant is not.
