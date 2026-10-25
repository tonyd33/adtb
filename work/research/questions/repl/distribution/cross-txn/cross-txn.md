---
title: How a transaction spanning partitions commits
pick: one
raised-by: [repl/distribution/by-key, repl/distribution/by-sum]
options:
  distributed-ssi:
    title: Two-phase commit with SSI conflict tracking across partitions
  deterministic:
    title: A sequencer orders registered transactions whose sets the compiler derives
    needs: [engine/deterministic-sets/registered-txn]
    links:
      - {rel: prior-art, to: prior/repl-calvin}
  central-resolver:
    title: Partitions store data; one resolver checks read and write sets at commit
    links:
      - {rel: prior-art, to: prior/engine-fdb}
---
A retag under `by-sum` stays local, but an FK check, a join or an
insert into two sums does not.

## repl/distribution/cross-txn/distributed-ssi

The settled concurrency control stretched across nodes; rw-conflicts
must be reported across partitions (unverified that any shipped system
does distributed SSI; CockroachDB uses timestamp ordering with
read refresh, unverified).

## repl/distribution/cross-txn/deterministic

Calvin: registered transactions are known before they run, so their
constructor relations and key ranges are known too. No 2PC and no
distributed aborts, but the accepted SSI would be replaced, and ad hoc
transactions need OLLP reconnaissance.

## repl/distribution/cross-txn/central-resolver

FoundationDB: optimistic validation of key ranges at commit by
resolvers, MVCC reads at a read version. Serializable, with the
constructor-scoped read sets registered queries already state.
