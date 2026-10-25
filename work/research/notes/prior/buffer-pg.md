---
title: Postgres shared buffers, bgwriter and checkpoints
---
Clock sweep over shared buffers with a usage count capped at 5 (ARC
was dropped in 8.0.2 over a pending patent, then 2Q, then clock in
8.1). Ring buffers for seq scans (256 KB), VACUUM and bulk writes.
bgwriter cleans buffers ahead of the clock hand. Buffered I/O through
the OS cache by default (shared_buffers about 25% of RAM); O_DIRECT only
via debug_io_direct; AIO since 18 with io_method worker or io_uring.
Spread checkpoints (completion target 0.9), full-page images on first
change after a checkpoint, leader-follower group commit.
https://github.com/postgres/postgres/blob/master/src/backend/storage/buffer/README
https://www.postgresql.org/docs/current/wal-configuration.html
https://www.postgresql.org/docs/current/runtime-config-wal.html
