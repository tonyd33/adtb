---
created-at: 2026-10-30
status: open
kind: bug
priority: 560
summary: "verify reruns from its pages phase on each block; polling jobs can be delayed without bound; a deferrable read with a command and verify_without_transaction wait on a worker, the second ignoring deadline and cancel; a stop in BTree::bulk causes a needless sweep; undo_overwrite always restores Checksum"
---

# Command wait follow ups

## Report
