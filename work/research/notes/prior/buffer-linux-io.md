---
title: Linux direct I/O, sync and io_uring semantics
---
O_DIRECT alignment varies by file system (statx STATX_DIOALIGN since
6.1; usually the device logical block). fsync does not persist a new
directory entry; the directory needs its own fsync. pwritev2
RWF_ATOMIC (6.11) gives torn-write protection for power-of-two,
naturally aligned O_DIRECT writes within device limits. io_uring
fsync is not ordered against in-flight writes; order with linked SQEs,
drain, or by waiting for completions.
https://man7.org/linux/man-pages/man2/open.2.html
https://man7.org/linux/man-pages/man2/pwritev2.2.html
https://man7.org/linux/man-pages/man2/io_uring_enter.2.html
