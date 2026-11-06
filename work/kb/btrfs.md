# btrfs

kind: system
home: https://btrfs.readthedocs.io

## Integrity

- Generation (transid): a counter updated each transaction; a metadata block written by copy-on-write stores the current generation, so blocks too new, and hence possibly inconsistent, can be identified. [V 2026-11-05] https://btrfs.readthedocs.io/en/latest/Glossary.html
- A tree block pointer carries the expected generation of the child; a child whose header generation differs fails with "parent transid verify failed", which catches a lost write of the child. [U] from memory
