---
title: Settling a status page that lost a write reads a commit whose record is recycled as aborted
pick: any
raised-by: [[mvcc/xid-clog, storage/readin-gate/recycle]]
options:
  refuse:
    title: Refuse a status the retained log cannot prove, as damage, instead of settling it aborted
  keep-evidence:
    title: Before recycling, keep each status the removed segments state somewhere the open reads
  live:
    title: Settle aborted as today
    excludes: [mvcc/settle-recycled/refuse, mvcc/settle-recycled/keep-evidence]
---
The open settles every id still in progress on a commit log status page
from the retained log: committed when the log holds its commit record,
aborted otherwise (mvcc.md §4.2). A status page that lost a write can
hold an id in progress whose commit record lies past the page's LSN.
When that record is in a recycled segment, the id reads as aborted, the
status page standing as authoritative. The lost write is detected (the
page is recorded as settled damage), but an acknowledged commit is
silently dropped: its rows read as never written.

Before the in-session recycle amendment of `storage/readin-gate/recycle`
this lost only commits of sessions before the last. With the checkpointer
recycling within a session, it loses any commit older than the floor.

Which ids are at risk: only those on a status page refused stale (its LSN
below the one the pager expects of it, from the retained log or the
fold), whose LSN is below the first retained record. On an intact page an
id in progress has no commit: the status change and the commit record
lie in one batch, so a commit would be on the page.

## mvcc/settle-recycled/refuse

A stale status page whose LSN is below the first record of the retained
log, and that holds an id in progress, cannot be settled: the log that
would prove each id's status is gone. The open records it as damage and
quarantines as the damage rules map commit log damage: the whole
database (mvcc.md §4.2), as for a retained log too damaged to settle.
The `verify` and `dump` paths of a quarantined database apply; repair
needs the archive or a backup holding the recycled segments.

```text
settle(page):
  if page.stale and page.lsn < retained_start and page.has_in_progress:
    record damage(page); quarantine whole   -- today: settle each id aborted
  else:
    settle from the retained log as today
```

Cost: a lost write of a status page whose log is recycled no longer opens
for transactions. Precedent: none checked.

## mvcc/settle-recycled/keep-evidence

Before a recycle removes segments, make every status they state durable
somewhere the open reads, so no status depends on a removed record. Ways,
none checked against the code:

- log a full image of each status page whose last record the removed
  segments hold, after the fold and before the removal, so the retained
  log names the page at an LSN past the recycled stretch and holds its
  content;
- force each such page and read it back before the removal, so the
  file's copy holds every recycled status (a later lost write of it is a
  later change, whose record the retained log holds);
- fold the committed ids of the removed segments into a persisted
  structure the settle reads, as the page LSN map is folded.

Cost: work at each recycle, bounded by the status pages the removed
segments change. A lost write of the page between this step and the next
change is still possible under the second way.

## mvcc/settle-recycled/live

What mvcc.md §4.2 states today: the id reads as aborted, the page is
recorded as settled damage, and `verify` reports the lost write once.
