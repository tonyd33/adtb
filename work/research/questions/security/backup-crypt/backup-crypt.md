---
title: How encryption carries into backups
pick: one
raised-by: [security/encrypt/page, security/encrypt/file-stream]
options:
  as-stored:
    title: Backups copy encrypted pages and log as stored; restore needs the key versions they use
    links:
      - {rel: prior-art, to: repl/backup/base-plus-log}
  rewrap:
    title: Backups are re-encrypted under a backup key
  plain:
    title: Backups are plaintext unless the backup tool encrypts them
    links:
      - {rel: prior-art, to: prior/security-cockroach-ear}
---
CockroachDB's encryption at rest does not cover its backups.

## security/backup-crypt/as-stored

A page copy plus log is already ciphertext; retiring a data key must
keep it available while any retained backup uses it.

## security/backup-crypt/rewrap

The backup decrypts and re-encrypts each page; incremental backups by
page LSN still work.

## security/backup-crypt/plain

A stolen backup is a stolen database.
