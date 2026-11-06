# ZFS

kind: system
home: https://openzfs.org

## Integrity

- A block's checksum is stored in the parent block pointer that references it, not in the block, so a misdirected or lost write fails verification on read; each block pointer also records the birth txg the block was written in. [S 2026-11-05] https://web.cecs.pdx.edu/~dmcgrath/courses/Su26/forensics/zfs.html
- A block that fails verification with no redundant copy is added to the pool's persistent error log, listed by `zpool status -v` as permanent errors; with a good copy it is repaired and not listed. [S 2026-11-05] https://docs.oracle.com/en/operating-systems/solaris/oracle-solaris/11.4/manage-zfs/zfs-data-corruption-errors.html
- The `head_errlog` feature extends the persistent error log entry with the birth txg of the error block. [S 2026-11-05] OpenZFS commit 431083f75bdd3efaee992bdd672625ec7240d252
