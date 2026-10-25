---
title: A page dirtied over and over pins the log behind it
pick: any
raised-by: [buffer/checkpoint/spread, buffer/checkpoint/fuzzy]
options:
  flush-oldest:
    title: When log space nears a limit, flush the frames with the oldest recLSN
  stall:
    title: Throttle writers until the checkpoint catches up
  live:
    title: Live with unbounded log growth
    excludes: [buffer/checkpoint/log-pin/flush-oldest, buffer/checkpoint/log-pin/stall]
---
A union index root or a hot constructor heap's last page stays dirty;
clock never evicts it, so its `rec_lsn` never advances and the log
cannot be truncated past it. Recovery time grows with it.

## buffer/checkpoint/log-pin/flush-oldest

InnoDB's checkpoint age: flushing is driven by `current_lsn -
min(rec_lsn)` against log capacity, and becomes synchronous near the
limit (unverified thresholds). The flushed page gets a new `rec_lsn`
on its next change.

## buffer/checkpoint/log-pin/stall

WiredTiger throttles application threads at `eviction_dirty_trigger`
(20% dirty). https://source.wiredtiger.com/develop/tune_cache.html

## buffer/checkpoint/log-pin/live

Disk fills with log.
