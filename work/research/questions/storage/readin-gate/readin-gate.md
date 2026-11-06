---
title: What every page read-in is checked against to catch a lost write
pick: some
raised-by: [buffer/faults/stale-page/lsn-check]
options:
  open-map:
    title: Build a per-page last-LSN map from the retained log at every open; every read-in checks it
    assessment: {grade: costly, why: "a scan of the whole retained log at each open, and the retained log grows without bound today"}
  persisted-map:
    title: Persist a per-page last-LSN map in the file, maintained with the pages and folded at checkpoint
    assessment: {grade: costly, unverified: true, why: "a new structure on every write path; its own lost write needs a defence"}
    links:
      - {rel: rivals, to: storage/readin-gate/open-map}
  recycle:
    title: Bound the retained log by recycling segments behind the checkpoint and the oldest backup's need
    assessment: {grade: costly, unverified: true, why: "makes open-map bounded; narrows verify's reach to the bounded log, which catalog.md section 11 already allows"}
  evicted-holes:
    title: Keep the bounded evicted table and close the specific holes (backup reads, frees)
    assessment: {grade: hack, why: "a page read after an open or after its slot is displaced is still served stale"}
forbid:
  - all: [storage/readin-gate/evicted-holes, storage/readin-gate/open-map]
    why: open-map replaces the evicted table
  - all: [storage/readin-gate/evicted-holes, storage/readin-gate/persisted-map]
    why: persisted-map replaces the evicted table
---
The accepted ruling `buffer/faults/stale-page/lsn-check` says redo and reads
compare a page's LSN with the LSN the log expects. Today only recovery
(from the redo LSN), `verify` and scrub (`read_log` over the whole retained
log) do. A read-in (`Pager::resident`, storage/pager.rs) checks
`past_log_end` and `lost_write`, which knows only a page's resident copy or
its slot in `EvictedLsns` (storage/evicted.rs): direct-mapped, `pid &
(len - 1)` over `4 * pool_frames` slots rounded up to a power of two
(4096 at the default 1024 frames), in memory, empty at every open. A page
whose slot another eviction took, or any page read after an open whose
last record lies before the redo LSN, is served stale. Vacuum prunes and
frees from it, a stale directory leaf names a freed page, dump reads old
rows, and backup copies it as sound (damage-readin-gate,
stale-leaf-names-freed-page, vacuum-erases-lost-write,
backup-list-misses-late-stale).

Contracts: storage.md §3's lost-write row names recovery, verify, scrub and
the bounded table (§10.3). catalog.md §11 says a lost write whose log has
left the retained log, on a page recovery does not read, is not detected,
and §11.2 says a page detected by its log position "on any read"
quarantines its holder. storage.md §6 lets segments before the redo LSN be
recycled once replication, archiving and change consumers no longer need
them; nothing in the code recycles (`clear_tail` removes only segments past
the end; the only other `drv.remove` is create's), so the retained log is
every byte ever logged.

What `read_log` costs now (storage/verify.rs, storage/log.rs): it lists and
opens every segment, reads the whole log sequentially with direct I/O in
64 KiB chunks on one thread, copies each 512-byte block into a fresh `Vec`
(`Slot::Bytes`), checks each block's CRC-32C, decodes every record
including full-page images, and keeps a `BTreeMap<Pid, Lsn>` of the last
record per page plus the broken chains. Memory is one entry per page the
log names, bounded by the file's pages. Time is linear in the log's
length, unrelated to the file's size. Recovery reads from the batch of the
redo LSN twice; scrub reads the whole log once per run; the commit-log
settle at open reads from the oldest in-progress status page's LSN.

Log sizes on disk from earlier runs (`find / -xdev -name '*.adt-log'`,
nonzero bytes of the last segment as a floor):

| workload | data file | segments | retained log |
| --- | --- | --- | --- |
| benchmark-feasibility lifecycle n=10000 | 10.0 MiB | 7 of 16 MiB | about 98 MiB |
| sim real_file reopen (shop) | 2.3 MiB | 12 of 256 KiB | about 2.8 MiB |
| calls-db two-statements | 1.0 MiB | 6 of 256 KiB | about 1.3 MiB |
| campaign.adt (Oct 28) | 6.7 MiB | 1 of 16 MiB | at least 0.7 MiB |

The log is about ten times the file after one write-heavy benchmark run and
grows with every write. Sim workloads keep their log in memory; their sizes
need a run.

Prior art (work/kb/): Oracle detects lost writes on a standby or in media
recovery by logging block reads (`DB_LOST_WRITE_PROTECT`), and since 18c by
a shadow tablespace holding each tracked block's SCN, compared on read,
itself ordinary redo-logged blocks, sized at about 2% of the data
(oracle-database.md). ZFS and btrfs keep the expected checksum or
generation in the parent pointer, so the referrer catches a stale child
(zfs.md, btrfs.md; `layout/checksum/in-parent` here). PostgreSQL and InnoDB
detect lost writes only during redo, by page LSN (postgres.md, innodb.md).

## storage/readin-gate/open-map

At open, after recovery, scan the retained log as `read_log` does and keep
`last: Pid -> Lsn`; maintain it on every page change (the LSN is the
page's new LSN) and drop `EvictedLsns`. `lost_write` with no resident image
compares with `last`; every read-in, backup copy and free checks one map.
A log with a gap clears the map at the gap, as `read_log` does, so pages
whose last record precedes a gap are unchecked.

- Cost: one extra full log scan per open (about 98 MiB on the benchmark
  above, linear in write volume ever); memory one entry per page the log
  names. Recovery could build it in its own first scan only if that scan
  started at the log's first segment instead of the redo LSN's batch.
- Verify and scrub can read the same map instead of rereading the log,
  keeping only the chain check.
- Changes storage.md §3's lost-write row to every read-in while the
  retained log holds the page's record, and §10.3's table.

## storage/readin-gate/persisted-map

A structure in the file holding each page's expected LSN: written in the
same batch as the page's change (a second page per change), or kept in
memory and folded at checkpoint, with records past the checkpoint replayed
into it at open (so open scans only from redo, which recovery already
reads). Oracle's shadow tablespace is this design. Detection then outlives
the log. Cost: about 8 bytes per page (0.1% of the file at 8 KiB pages),
plus either a second page write per change or a checkpoint fold. The map's
own pages can lose a write (storage/readin-gate/map-lost-write).

## storage/readin-gate/recycle

Remove segments wholly before `min(redo LSN, oldest backup's need,
replication and archive consumers)`. With open-map this bounds the open scan
to roughly a checkpoint interval plus what backups pin. It narrows what
`verify` can check to the bounded log, which catalog.md §11 already states,
and loses detection for a page whose last record is recycled
(storage/readin-gate/backup-retention). Needed anyway: the log grows
without bound.

## storage/readin-gate/evicted-holes

Keep `EvictedLsns`, have backup and frees consult `read_log`'s map, widen
the table or make it exact for pages evicted this open. Pages first read
after an open stay unchecked, so a lost write found by nothing but the
retained log is served until verify or scrub runs.
