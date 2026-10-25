# Lehman and Yao, B-link tree

kind: paper
home: https://doi.org/10.1145/319628.319663

## Algorithm

- "Efficient Locking for Concurrent Operations on B-Trees", TODS 6(4), Dec 1981, pp 650-670: right-link and high key per node; searches take no read locks and move right on a concurrent split; inserts lock at most a few nodes, coupling during ascent (child, parent, parent's right sibling at most); assumes fixed-size keys and unshared in-memory copies of pages. [S 2026-10-25] as summarised in https://raw.githubusercontent.com/postgres/postgres/master/src/backend/access/nbtree/README
