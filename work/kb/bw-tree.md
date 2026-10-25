# Bw-tree and OpenBw-Tree

kind: paper
home: https://www.cs.cmu.edu/~huanche1/publications/open_bwtree.pdf

## Design and critique

- Bw-tree (Levandoski, Lomet, Sengupta, ICDE 2013) is lock-free: updates prepend delta records to a node, and a Mapping Table from logical node id to physical address is updated with compare-and-swap; delta chains are consolidated into a new base node. Used in SQL Server Hekaton. [V 2026-10-25] Wang et al. SIGMOD 2018, https://www.cs.cmu.edu/~huanche1/publications/open_bwtree.pdf
- Wang et al. 2018: OpenBw-Tree beats the original design by 1.1-2.5x on insert-heavy and 1.1-1.4x on read-heavy workloads, yet is "considerably slower" than SkipList, Masstree and ART, and slower than a B+tree with optimistic lock coupling; "lock-freedom does not always pay off". Default max delta chain 2 (inner) and 24 (leaf). Shrinking the Mapping Table needs all workers blocked. [V 2026-10-25] same
