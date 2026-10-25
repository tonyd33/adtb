# OrioleDB

kind: system
home: https://github.com/orioledb/orioledb

## Storage and MVCC

- Postgres table access method. Index-organized tables: primary B+-tree leaves are rows; secondary index entries carry secondary key plus primary key; no TID. [V 2026-10-25] https://github.com/orioledb/orioledb/blob/main/doc/architecture/overview.mdx
- MVCC via undo: tuple headers form lists with the head on the data page and older entries in the undo log; update records hold header and body, delete and lock records only a header. Undo records also chain per transaction for abort. Block-level undo records (compact, split, merge) hold page images. Undo written to disk only at checkpoints or on overflow of shared memory. [V 2026-10-25] same file
- No VACUUM; 64-bit transaction ids by default, so no wraparound. [V 2026-10-25] https://github.com/orioledb/orioledb README
- Snapshot is a CSN (commit sequence number) plus xlog pointer, xmin and cid; an xid map gives each OXid's csn. [V 2026-10-25] https://github.com/orioledb/orioledb/blob/main/include/transam/oxid.h
- No SSI predicate locks: SERIALIZABLE on OrioleDB tables takes a table ExclusiveLock (default), errors, or downgrades to REPEATABLE READ (orioledb.serializable). [V 2026-10-25] oxid.h
- Primary key update is delete plus insert; a concurrent updater waiting on the row gets "tuple to be locked has its primary key changed due to concurrent update". A delete then reinsert of the same key is followed as a new version by a waiting updater. [V 2026-10-25] https://github.com/orioledb/orioledb/blob/main/doc/architecture/row-level-concurrency.mdx
- Row-level WAL; secondary index trees are not WAL-logged, rebuilt from primary and TOAST changes on recovery; copy-on-write fuzzy checkpoints. [V 2026-10-25] overview.mdx
