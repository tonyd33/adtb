---
title: How audit and change history are made tamper-evident
pick: one
raised-by: [security/audit/auth, security/audit/ops, security/audit/calls, security/audit/reads, security/audit/writes]
options:
  none:
    title: Not tamper-evident
  chain:
    title: Each record hashes its predecessor; the head is exported periodically
  merkle:
    title: A Merkle history tree over records, with logarithmic proofs
    links:
      - {rel: prior-art, to: prior/security-crosby-wallach}
  ledger:
    title: Every commit's change set and audit records are hashed into blocks chained by hash
    links:
      - {rel: prior-art, to: prior/security-sql-ledger}
---
An administrator who can edit the file can edit history. Tamper
evidence detects, never prevents, and depends on a digest kept
elsewhere.

## security/tamper/none

Audit records are as trustworthy as the file holding them.

## security/tamper/chain

```
$ adt audit digest      -- seq 991204 head 3f9a...
$ adt audit verify --from-digest digests/2026-10.txt
```

A proof that one record is present needs the chain from it to the head.

## security/tamper/merkle

Crosby and Wallach: membership and consistency proofs between two
digests in logarithmic size.

## security/tamper/ledger

SQL Server ledger: rows per transaction in a Merkle tree, transactions
into blocks, blocks chained; digests to WORM storage. Here the commit
stream is the redo log's commit records and the operator log, so a
digest covers data, schema and audit together.
