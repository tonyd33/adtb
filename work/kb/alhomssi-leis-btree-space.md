# Alhomssi and Leis, Contention and Space Management in B-Trees

kind: paper
home: https://www.cidrdb.org/cidr2021/papers/cidr2021_paper21.pdf

## Space and splits

- CIDR 2021. Textbook B-trees guarantee 50% fill; random inserts give about 70%, sorted inserts stay at 50%; industrial B-trees free a node only when completely empty rather than merging at half-empty. [V 2026-10-25] https://www.cidrdb.org/cidr2021/papers/cidr2021_paper21.pdf
- XMerge: merge a group of X immediate siblings whose total free space is at least one node into X-1 nodes, compacting tuples rightward and freeing the leftmost; X tried from 2 to K, K=5 found good; parent exclusively latched during compaction. [V 2026-10-25] same
- Contention Split: detect write contention from different threads updating different tuples of one node, and split it with a separator between the hot tuples. [V 2026-10-25] same
