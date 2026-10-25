---
title: Where a page's nonce comes from
pick: one
raised-by: [security/encrypt/page]
options:
  random:
    title: A fresh random nonce per page write, stored in the page
    links:
      - {rel: prior-art, to: prior/security-sqlcipher}
  lsn:
    title: Page id and page LSN, so every write that changes a page changes its nonce
    needs: [mvcc/status/clog-lookup]
    links:
      - {rel: prior-art, to: prior/security-pg-tde}
  xts:
    title: AES-XTS tweaked by page id, with no stored nonce
---
A counter-mode or GCM nonce must never repeat under one key.

## security/nonce/random

SQLCipher stores a random IV per page write in the page's tail. Costs
reserved bytes in every page.

## security/nonce/lsn

Postgres TDE stalled on LSN reuse: hint bits change pages without new
WAL. Here reads never write pages (`mvcc/status/clog-lookup`), so every
page change goes through the log, provided every other unlogged page
write (pruning on access, kill bits on index entries) also takes an LSN.

## security/nonce/xts

Disk-encryption mode (IEEE 1619, unverified). No integrity, and equal
plaintext at one page id gives equal ciphertext, so an attacker
watching the file sees which 16-byte blocks changed.
