# InnoDB

kind: system
home: https://dev.mysql.com/doc/refman/8.4/en/innodb-storage-engine.html

## Row formats

- COMPACT and DYNAMIC records have a 5-byte header, preceded by a variable-length header with a null bitmap and a length per non-null variable field (1 byte, or 2 when the column can exceed 255 bytes and the value exceeds 127, or is partly off-page). [V 2026-10-25] https://dev.mysql.com/doc/refman/8.4/en/innodb-row-format.html
- Clustered index records carry DB_TRX_ID (6 bytes) and DB_ROLL_PTR (7 bytes), plus DB_ROW_ID (6) without a primary key. [V 2026-10-25] https://dev.mysql.com/doc/refman/8.4/en/innodb-row-format.html
- COMPACT keeps a 768-byte prefix of long columns in the record and the rest on overflow pages; DYNAMIC stores long columns fully off-page behind a 20-byte pointer, and keeps TEXT/BLOB of 40 bytes or less inline. Max row about half a page. [V 2026-10-25] https://dev.mysql.com/doc/refman/8.4/en/innodb-row-format.html
- innodb_page_size 4, 8, 16 (default), 32 or 64 KB, fixed at instance initialization. [V 2026-10-25] https://dev.mysql.com/doc/refman/8.4/en/innodb-row-format.html

## Torn pages

- Doublewrite buffer: pages are first written to #ib_<page_size>_<n>.dblwr files in large sequential chunks with one fsync, then to their place; recovery takes a good copy from there. Modes ON/DETECT_AND_RECOVER, DETECT_ONLY (metadata only), OFF. Disabled automatically on Fusion-io atomic-write devices. [V 2026-10-25] https://dev.mysql.com/doc/refman/8.4/en/innodb-doublewrite-buffer.html

## Multi-versioning

- Clustered index records updated in place; DB_ROLL_PTR points at the undo record in a rollback segment that rebuilds the prior version. Delete is an update setting a deleted bit. [V 2026-10-25] https://dev.mysql.com/doc/refman/8.4/en/innodb-multi-versioning.html
- Insert undo discarded at commit; update undo kept until no snapshot could need it. Long transactions, including read-only consistent reads, stop undo discard and grow the rollback segment. [V 2026-10-25] same page
- Secondary index records carry no system columns and are not updated in place: an update of an indexed column delete-marks the old record and inserts a new one; purge removes delete-marked records. A read hitting a delete-marked record or a page updated by a newer transaction goes to the clustered index to check DB_TRX_ID, and covering-index reads are not used then. [V 2026-10-25] same page
- Purge physically removes a deleted row and its index records when it discards the delete's update undo. innodb_purge_threads default 1 (<= 16 logical CPUs) else 4, max 32; innodb_purge_batch_size default 300 undo pages; innodb_max_purge_lag default 0 (no throttling), delay ((lag/max) - 0.9995) * 10000 microseconds; history list length usually under a few thousand. [V 2026-10-25] https://dev.mysql.com/doc/refman/8.4/en/innodb-purge-configuration.html

## Buffer pool and checkpoints

- LRU with midpoint insertion: new pages enter at the head of the old sublist, 3/8 of the pool by default (innodb_old_blocks_pct, range 5 to 95); innodb_old_blocks_time (default 1000 ms) keeps a page old until accessed again after that delay, for scan resistance. [V 2026-10-25] https://dev.mysql.com/doc/refman/8.4/en/innodb-performance-midpoint_insertion.html https://dev.mysql.com/doc/refman/8.4/en/innodb-buffer-pool.html
- Fuzzy checkpointing: dirty pages flushed in small batches; recovery scans the log forward from the checkpoint label. [V 2026-10-25] https://dev.mysql.com/doc/refman/8.4/en/innodb-checkpoints.html
- Doublewrite writes in one sequential chunk with one fsync, except with innodb_flush_method O_DIRECT_NO_FSYNC. [V 2026-10-25] https://dev.mysql.com/doc/refman/8.4/en/innodb-doublewrite-buffer.html
