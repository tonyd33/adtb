# SQLite

kind: system
home: https://www.sqlite.org/

## Concurrency

- WAL mode: concurrent readers and one writer; a read transaction sees a snapshot from its start; upgrading a stale read transaction to write fails with SQLITE_BUSY_SNAPSHOT. [V 2026-10-25] https://www.sqlite.org/isolation.html
- WAL mode: commit appends a commit record to the `-wal` file; readers remember an end mark and look pages up in the shared-memory wal-index (`-shm`); readers do not block the writer and the writer does not block readers; one writer at a time. [V 2026-10-25] https://www.sqlite.org/wal.html
- WAL requires all processes on one host (shared memory); transactions across ATTACHed databases are not atomic in WAL mode. [V 2026-10-25] https://www.sqlite.org/wal.html
- Checkpoint copies WAL pages back into the database file, stopping at pages past any active reader's end mark; automatic checkpoint at 1000 pages by default. [V 2026-10-25] https://www.sqlite.org/wal.html
- Rollback-journal mode locks the whole file through five states: UNLOCKED, SHARED, RESERVED (one writer intends to write, readers continue), PENDING (blocks new SHARED), EXCLUSIVE (needed to write). No row or page locks. [V 2026-10-25] https://www.sqlite.org/lockingv3.html
- A hot journal (exists, well-formed header, no RESERVED lock held) is rolled back by the next reader before it reads. [V 2026-10-25] https://www.sqlite.org/lockingv3.html

## Architecture

- Layers: interface, tokenizer, Lemon-generated parser, code generator, VDBE bytecode engine, B-tree (`btree.c`, one B-tree per table and per index), pager (page cache, rollback and atomic commit), VFS (OS file abstraction). [V 2026-10-25] https://www.sqlite.org/arch.html

## Storage and file format

- Database is one file of fixed-size pages; page size a power of two from 512 to 65536; at most 2^32-2 pages. 100-byte header on page 1 beginning "SQLite format 3\0". [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- Header holds the schema cookie (offset 40), incremented on every schema change so prepared statements re-prepare, and the schema format number 1 to 4 (offset 44). [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- Two B-tree kinds: table B-trees (B+tree keyed by signed 64-bit rowid, data only in leaves) and index B-trees (arbitrary keys, no data). Page type byte: 0x02 interior index, 0x05 interior table, 0x0a leaf index, 0x0d leaf table. Other pages: freelist trunk/leaf, overflow, pointer map. [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- WITHOUT ROWID tables are stored as an index B-tree keyed by the PRIMARY KEY columns followed by the remaining columns. [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- Record format: varint header size, one varint serial type per column, then values. Serial types: 0 NULL; 1-6 big-endian ints of 1,2,3,4,6,8 bytes; 7 float64; 8 and 9 the constants 0 and 1; 10,11 reserved; even N>=12 BLOB of (N-12)/2 bytes; odd N>=13 TEXT of (N-13)/2 bytes. Each value carries its own type tag. [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- Schema is the table `sqlite_schema(type, name, tbl_name, rootpage, sql)` rooted at page 1; `sql` holds the original CREATE text, which is re-parsed on open. [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- Rollback journal stores original page images (page number, content, checksum seeded by a random nonce). WAL frames carry a 24-byte header with salts and cumulative checksums; a frame is valid only if salts match and the checksum chain holds up to it. [V 2026-10-25] https://www.sqlite.org/fileformat2.html

## Types

- Five storage classes: NULL, INTEGER, REAL, TEXT, BLOB. Typing is dynamic: the type belongs to the value, not the column. [V 2026-10-25] https://www.sqlite.org/datatype3.html
- Columns have one of five affinities (TEXT, NUMERIC, INTEGER, REAL, BLOB) derived from the declared type name by substring rules ("INT", then "CHAR"/"CLOB"/"TEXT", then "BLOB" or none, then "REAL"/"FLOA"/"DOUB", else NUMERIC); affinity coerces but does not restrict. [V 2026-10-25] https://www.sqlite.org/datatype3.html
- No boolean or date storage class; booleans are 0/1, dates are TEXT, REAL or INTEGER. [V 2026-10-25] https://www.sqlite.org/datatype3.html
- No enum or sum type; the usual encoding is TEXT or INTEGER with CHECK constraints or a lookup table. [U]
- STRICT tables (3.37.0, 2021-11-27) allow only INT, INTEGER, REAL, TEXT, BLOB, ANY; lossy coercion raises `SQLITE_CONSTRAINT_DATATYPE`; ANY stores values unchanged. Older versions cannot read a STRICT schema. [V 2026-10-25] https://www.sqlite.org/stricttables.html

## Query execution (VDBE)

- `sqlite3_prepare_v2()` compiles SQL to a bytecode program; `sqlite3_step()` runs it. The VM is register-based; each instruction is opcode plus P1-P3 (i32), P4 (variable type), P5 (u16 flags). About 191 opcodes; opcodes are not a stable API. [V 2026-10-25] https://www.sqlite.org/opcode.html
- Programs operate on numbered B-tree cursors (`OpenRead`, `OpenWrite`, `Column`, `Next`, `Seek*`). `EXPLAIN` prints the program. [V 2026-10-25] https://www.sqlite.org/opcode.html
- Stated reasons for bytecode over a tree of operator objects: readable EXPLAIN, a clean front-end/back-end split, pausing after each row without unwinding a stack, smaller prepared statements. Stated cost: plans cannot adapt mid-query and dataflow parallelism is harder. [V 2026-10-25] https://www.sqlite.org/whybytecode.html

## Evolution

- ALTER TABLE supports only RENAME TABLE, RENAME COLUMN, ADD COLUMN, DROP COLUMN, and (3.53.0+) ALTER COLUMN for NOT NULL. [V 2026-10-25] https://www.sqlite.org/lang_altertable.html
- ADD COLUMN does not rewrite rows (time independent of table size); it forbids PRIMARY KEY, UNIQUE, non-constant defaults, NOT NULL without non-NULL default, and STORED generated columns. [V 2026-10-25] https://www.sqlite.org/lang_altertable.html
- A record with fewer values than the table has columns reads the missing trailing columns as their defaults, which is how ADD COLUMN avoids a rewrite. [U]
- DROP COLUMN fails if the column is in a primary key, UNIQUE, an index, a partial index WHERE, CHECK, FOREIGN KEY, a generated column, a trigger or a view. [V 2026-10-25] https://www.sqlite.org/lang_altertable.html
- Other changes use the documented 12-step procedure: create new table, copy rows, drop old, rename new into place, recreate indexes, triggers and views, check foreign keys, all in one transaction. [V 2026-10-25] https://www.sqlite.org/lang_altertable.html
- ALTER TABLE is limited because the schema is stored as CREATE text in `sqlite_schema`; ALTER edits that text and re-parses the whole schema. [V 2026-10-25] https://www.sqlite.org/lang_altertable.html

## File format

- One file; 100-byte header at offset 0: magic "SQLite format 3\0", page size u16 at 16 (512 to 32768, value 1 means 65536), read/write format versions at 18/19 (2 = WAL), reserved bytes per page at 20, file change counter at 24, size in pages at 28, first freelist trunk at 32, freelist count at 36, schema cookie at 40, schema format number at 44 (1 to 4), text encoding at 56. All multi-byte integers big-endian except the WAL-index. [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- B-tree page header 8 bytes (leaf) or 12 (interior): type flag (0x02, 0x05, 0x0a, 0x0d), first freeblock u16, cell count u16, cell content start u16, fragmented free bytes u8 (at most 60), right-most child u32 on interior pages; then an array of 2-byte cell offsets in key order. [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- Record: varint header size, one serial-type varint per column, then values. Serial types: 0 NULL, 1..6 ints of 1,2,3,4,6,8 bytes, 7 float64, 8 and 9 the constants 0 and 1, N>=12 even blob of (N-12)/2, N>=13 odd text of (N-13)/2. Varint 1 to 9 bytes big-endian, 7 bits per byte, 9th byte 8 bits. [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- Overflow: with usable size U, table-leaf payload P is all local when P <= U-35; else local part is K = M + ((P-M) % (U-4)) if K <= X, else M = ((U-12)*32/255)-23; overflow pages start with a 4-byte next-page number. [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- Free pages: trunk pages list leaf free pages (4-byte page numbers); auto-vacuum keeps pointer-map pages of 5-byte entries (type, parent page). Lock-byte page at offset 2^30 is never used. [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- WAL frames: 24-byte header (page number, commit size, two salts, two checksums) then one page image; rollback journal records hold page number, original page and a 32-bit checksum. [V 2026-10-25] https://www.sqlite.org/fileformat2.html

## B-tree file format

- B-tree page header 8 bytes (leaf) or 12 (interior): type u8 (0x02 interior index, 0x0a leaf index, 0x05 interior table, 0x0d leaf table), first freeblock u16, cell count u16, cell content start u16, fragmented bytes u8, right-most child u32 (interior only); then a 2-byte cell pointer array in key order. [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- Index leaf cell: varint payload size, payload, u32 first overflow page if spilled; interior cell adds a u32 left child first. Index payload stays on page up to X = ((U-12)*64/255)-23, min M = ((U-12)*32/255)-23. [V 2026-10-25] same
- An index key is a record of the indexed columns followed by the rowid (or the PRIMARY KEY columns for WITHOUT ROWID), so every index key is unique. Records compare column by column: NULL < numeric < text (collation) < blob (memcmp); the record header gives each column a serial type (0 NULL, 1-6 ints of 1,2,3,4,6,8 bytes, 7 float, 8/9 constants 0/1, even >=12 blob, odd >=13 text). [V 2026-10-25] same

## Pager durability details

- WAL header 32 bytes, eight big-endian u32: magic 0x377f0682/0x377f0683, format 3007000, page size, checkpoint sequence, salt-1 (incremented each checkpoint), salt-2 (random each checkpoint), two checksum words over the first 24 bytes. Frame header 24 bytes: page number, db size in pages for commit frames else 0, salt-1, salt-2, two cumulative checksum words. [V 2026-10-25] https://www.sqlite.org/fileformat2.html
- A reader takes the last valid copy of page P followed by or being a commit frame. [V 2026-10-25] same page
- Checkpoint starvation: a checkpoint resets the WAL only with no reader using it. [V 2026-10-25] https://www.sqlite.org/wal.html
- Powersafe overwrite assumed by default since 3.7.9: a write never damages bytes outside its range on power loss. Rollback journal pages carry a 32-bit checksum since the FS may reorder writes. [V 2026-10-25] https://www.sqlite.org/atomiccommit.html

## Backup

- Online backup API copies pages incrementally (`sqlite3_backup_step(n)`); a write from another connection mid-copy usually restarts it; the finished copy is a consistent snapshot as of start. `VACUUM INTO` writes a vacuumed copy of a live database. [V 2026-10-25] https://www.sqlite.org/backup.html

## Operations

- PRAGMA integrity_check(N|table) checks entry order, record format, missing pages, missing/surplus index entries, UNIQUE/CHECK/NOT NULL, freelist and page use; returns up to N (default 100) rows or 'ok'; not FKs (foreign_key_check). quick_check skips UNIQUE and index-to-table matching, O(N) not O(N log N). [V 2026-10-25] https://www.sqlite.org/pragma.html
- auto_vacuum NONE (default) keeps freed pages on the freelist and never shrinks; FULL truncates at commit; INCREMENTAL defers to incremental_vacuum(N). Switching from NONE needs a VACUUM. [V 2026-10-25] https://www.sqlite.org/pragma.html
- Side-effect-free pragmas are table-valued functions pragma_<name>(...) since 3.16.0. Some pragmas persist in the file header (user_version, application_id, auto_vacuum, page_size, WAL journal_mode); most are per connection (cache_size, busy_timeout, foreign_keys, synchronous). [V 2026-10-25] https://www.sqlite.org/pragma.html
- "Newer versions of SQLite can always read and/or write database files created by older versions of SQLite, back to version 3.0.0"; older versions read newer files unless they use newer features (WAL unreadable before 3.7.0). [V 2026-10-25] https://www.sqlite.org/formatchng.html

## Virtual tables and backup

- Virtual tables delegate access to callbacks; xBestIndex receives constraints (column, operator, usable), ORDER BY and colUsed, returns argvIndex/omit per constraint, idxNum/idxStr, estimatedCost, estimatedRows, orderByConsumed. No triggers, no CREATE INDEX, no added constraints on a virtual table. Eponymous-only virtual tables are table-valued functions with HIDDEN columns as parameters. xUpdate optional. FTS and R-Tree keep data in shadow tables. [V 2026-10-25] https://www.sqlite.org/vtab.html
- VACUUM INTO writes a minimal consistent snapshot to a new file; alternative to the backup API, which is incremental and preserves free pages. [V 2026-10-25] https://www.sqlite.org/lang_vacuum.html
- Loadable extensions disabled by default; enabled per connection with sqlite3_enable_load_extension. [U]

## Testing (testing.html)

- As of 3.42.0, about 155.8 KSLOC of library code and 590 times as much test code (92053.1 KSLOC). [V 2026-10-25] https://www.sqlite.org/testing.html
- Four independent harnesses: TCL tests, TH3 (proprietary C, 100% branch and 100% MC/DC coverage of the core, using only published interfaces), SQL Logic Test (SLT, compares answers with PostgreSQL, MySQL, SQL Server, Oracle 10g; 7.2 million queries), and dbsqlfuzz (proprietary, mutates SQL and database file together). [V 2026-10-25] same
- Anomaly tests: out-of-memory (instrumented malloc failing once or continuously), I/O error, crash and power-loss, compound failures. [V 2026-10-25] same
- Disabled-optimization tests: SQLITE_TESTCTRL_OPTIMIZATIONS turns optimizations off and the whole suite must give the same output. [V 2026-10-25] same
- ALWAYS()/NEVER() macros mark defensive conditions; pass-through in release, assert during testing. Mutation testing checks every branch makes a difference to output. [V 2026-10-25] same
- AFL used 3.8.10 to 3.29.0, then superseded; OSS-Fuzz since 2016. [V 2026-10-25] same

## Scope (whentouse.html, limits.html)

- Positioned as local storage for applications and devices, not a shared enterprise repository; client/server recommended for many concurrent writers (one writer at a time per file) and very large datasets. [V 2026-10-25] https://www.sqlite.org/whentouse.html
- Maximum database size 4294967294 pages, about 281 TB at 65536-byte pages. [V 2026-10-25] https://www.sqlite.org/limits.html
