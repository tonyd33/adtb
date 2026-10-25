# Megiddo, Modha: ARC

kind: paper
home: https://www.usenix.org/conference/fast-03/arc-self-tuning-low-overhead-replacement-cache

## Replacement

- FAST 2003, "ARC: A Self-Tuning, Low Overhead Replacement Cache": adaptively balances recency and frequency for uniform page sizes. [V 2026-10-25] https://www.usenix.org/conference/fast-03/arc-self-tuning-low-overhead-replacement-cache
- Keeps two LRU lists of resident pages (seen once, seen twice) plus two ghost lists of evicted ids, and moves the target split on ghost hits. [U]
- Related policies: LRU-K (O'Neil, O'Neil, Weikum SIGMOD 1993, evict by K-th most recent reference time); 2Q (Johnson, Shasha VLDB 1994, A1in FIFO, A1out ghost, Am LRU). [U]
