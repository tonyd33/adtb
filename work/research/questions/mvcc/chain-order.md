---
title: Which end of an in-heap version chain the indexes reach
pick: one
raised-by: [engine/in-heap]
options:
  o2n:
    title: Oldest to newest, each version links forward to its successor
    links:
      - {rel: prior-art, to: prior/mvcc-pg-heap}
      - {rel: gains, to: engine/retag-chain/forward-pointer}
  n2o:
    title: Newest to oldest, indexes repointed at the new head on every update
    links:
      - {rel: prior-art, to: prior/mvcc-wu-2017}
links:
  - {rel: prior-art, to: prior/mvcc-wu-2017}
---
With every version a tuple in its constructor's heap, the chain is
singly linked (a latch-free doubly linked list is not possible, Wu et
al. sect. 4.1). A retag always starts a new chain segment in another
heap, so either direction must name a constructor as well as a TID.

```
o2n:  index -> v1 (EmailPass, tid 1) -> v2 (EmailPass, tid 9) -> v3 (Google, tid 4)
n2o:  index -> v3 (Google, tid 4)    -> v2 (EmailPass, tid 9) -> v1 (EmailPass, tid 1)
```

## mvcc/o2n

Postgres: `t_ctid` of the old version names the new one; a reader that
wants the newest walks forward, an old snapshot stops early. Indexes
are untouched by an update that changes no key (with HOT, see
`mvcc/hot`). A waiter on a retagged row follows the link into the new
heap. Wu et al. found O2N the slowest ordering under contention, since
latest-version reads walk chains whose length depends on GC keeping up.

## mvcc/n2o

The head is the newest version; each version links back. Every update
inserts a new entry into every union index (or repoints an indirection
slot, `mvcc/row-id`). Wu et al.: N2O 2.4 to 3.4 times O2N at the
highest contention. A snapshot reader that reaches `(7, Google, tid 4)`
and finds it too new walks back into the `EmailPass` heap, so with N2O
the retag's old index entry is not needed for readers
(`engine/retag-snapshot`), only for uniqueness skipping.
