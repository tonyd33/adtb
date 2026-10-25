# Linux I/O interfaces

kind: system
home: https://man7.org/linux/man-pages/

## O_DIRECT

- Alignment restrictions on buffer address, length and file offset vary by file system and kernel and may be absent; misaligned I/O either fails with EINVAL or falls back to buffered. Since Linux 6.1 statx(2) STATX_DIOALIGN reports them. Since 2.6.0 the unit is the device logical block size (typically 512), found with BLKSSZGET or blockdev --getss. [V 2026-10-25] https://man7.org/linux/man-pages/man2/open.2.html

## fsync

- fsync also flushes inode metadata, but not the directory entry: a new or renamed file needs fsync on the directory fd. fdatasync skips metadata not needed to read the data back. [V 2026-10-25] https://man7.org/linux/man-pages/man2/fsync.2.html
- EIO may concern data written through another fd of the same file; since 4.13 writeback errors are reported to every fd that might have written the data. [V 2026-10-25] same page

## pwritev2 flags

- RWF_DSYNC (4.7): per-write O_DSYNC. RWF_ATOMIC (6.11): torn-write protection, all or none of the write after power loss; length a power of two within statx stx_atomic_write_unit_min..max, naturally aligned offset, at most stx_atomic_write_segments_max iovecs, O_DIRECT only, O_SYNC/O_DSYNC needed for durability. RWF_DONTCACHE (6.14) drops page cache after the op. [V 2026-10-25] https://man7.org/linux/man-pages/man2/pwritev2.2.html

## io_uring

- Shared submission and completion rings set up by io_uring_setup and mmap; IORING_SETUP_SQPOLL lets a kernel thread poll the SQ, and buffers must stay valid until completion. [V 2026-10-25] https://man7.org/linux/man-pages/man7/io_uring.7.html
- IORING_OP_FSYNC (IORING_FSYNC_DATASYNC for fdatasync semantics) is not ordered against in-flight writes: order with IOSQE_IO_LINK chains, IOSQE_IO_DRAIN, or by waiting for prior CQEs. [V 2026-10-25] https://man7.org/linux/man-pages/man2/io_uring_enter.2.html
