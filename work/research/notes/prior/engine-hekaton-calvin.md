---
title: Hekaton and Calvin
---
Hekaton, unverified (Larson et al. VLDB 2011; Diaconu et al. SIGMOD 2013): in-memory
optimistic MVCC, validation of reads and scans at commit, redo log of
committed versions only, indexes not logged and rebuilt on recovery.
https://vldb.org/pvldb/vol5/p298_per-akelarson_vldb2012.pdf (unverified)
Calvin (Thomson et al. SIGMOD 2012): a sequencer orders transactions
before execution; locks acquired in that order make execution
deterministic. Needs read and write sets in advance; dependent
transactions use a reconnaissance pass (OLLP).
https://cs.yale.edu/homes/thomson/publications/calvin-sigmod12.pdf (unverified)
