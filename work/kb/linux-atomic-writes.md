# Linux atomic writes

kind: system
home: https://lwn.net/Articles/974578/

## Torn-write protection

- pwritev2 RWF_ATOMIC added in Linux 6.11 with statx reporting min/max atomic unit; XFS and ext4 support from 6.13; requires O_DIRECT, power-of-two length naturally aligned, single iovec, initially at most one filesystem block (e.g. 16 KiB); needs device atomic support (NVMe AWUPF, SCSI ATOMIC WRITE(16)). [S 2026-10-25] https://lwn.net/Articles/1009298/ https://www.phoronix.com/news/Linux-6.11-Block-Atomic-Writes
- Survey: SQL Server detects torn pages with a 2-bit per-512-byte-sector counter; Oracle compares the SCN in header and tail; SQLite and Postgres log full page images; InnoDB, XtraDB, CedarDB double-write; LMDB copy-on-write; OrioleDB copy on first write after a checkpoint; AlloyDB Omni writes 8 KB with RWF_ATOMIC and disables full page writes. NVMe AWUPF is in logical blocks. [S 2026-10-25] https://transactional.blog/blog/2025-torn-writes
- Man page confirms RWF_ATOMIC since 6.11: power-of-two length within statx atomic unit bounds, naturally aligned offset, O_DIRECT only, needs O_SYNC/O_DSYNC for durability. [V 2026-10-25] https://man7.org/linux/man-pages/man2/pwritev2.2.html
