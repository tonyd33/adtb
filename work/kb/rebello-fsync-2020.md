# Rebello et al.: Can Applications Recover from fsync Failures?

kind: paper
home: https://www.usenix.org/system/files/atc20-rebello.pdf

## Findings

- USENIX ATC 2020. ext4, XFS and Btrfs all mark pages clean after a failed fsync, so retrying fsync does not rewrite the data. ext4 and XFS keep the new content in the page cache; Btrfs reverts it to the previous state. ext4 data=journal can report the failure on the next call instead. Failed journal-block writes make the file system unavailable. [V 2026-10-25] https://www.usenix.org/system/files/atc20-rebello.pdf sec. 1, 3
- Applications: Redis ignores fsync return codes; LMDB, LevelDB, SQLite revert in-memory state and report; PostgreSQL crashes and replays. None retry. LMDB gives false failures; Redis, LMDB, LevelDB, SQLite corrupt data; PostgreSQL can lose old and new data on updates. [V 2026-10-25] same paper sec. 1, 5 #7
- Finding #8: LevelDB, SQLite and PostgreSQL recover from the WAL through the page cache, which can hold content never written; recovery should read on-disk content. [V 2026-10-25] same paper sec. 5
- FreeBSD VFS re-dirties pages on write failure (except device removal); Linux leaves it to each file system. [V 2026-10-25] same paper sec. 5 #5
- The paper judges PostgreSQL's direct-I/O mode as possibly best at handling fsync failures. Tool: CuttleFS, a FUSE file system with its own user-space page cache emulating each file system's failure behaviour. [V 2026-10-25] same paper sec. 4, 5 #7
