---
title: Whether and where the engine encrypts at rest
pick: one
options:
  none:
    title: The engine writes plaintext; disk or filesystem encryption is the host's
  page:
    title: The pager encrypts each page as it is written and decrypts it as it is read
    links:
      - {rel: prior-art, to: prior/security-sqlcipher}
      - {rel: prior-art, to: prior/security-pg-tde}
  file-stream:
    title: Files are encrypted as byte streams in counter mode keyed by file offset
    links:
      - {rel: prior-art, to: prior/security-cockroach-ear}
---
One file per database (`layout/files/single-file`), direct I/O through
the engine's own pool (`buffer/io-path/direct`), a 32-bit checksum per
page (`layout/checksum/in-page-32`), and a redo log with full-page
images (`layout/torn/fpw`).

## security/encrypt/none

LUKS or a cloud volume key protects a stolen disk, not a copied file.

## security/encrypt/page

```
let db = adtb::open_with("app.db", Key::from_kms("kms://vault/adt-app"))?;
```

SQLCipher per page; Percona pg_tde per relation through a patched
storage manager. Pages are plaintext in the pool, ciphertext on disk.
Unlike pg_tde, the catalog lives in the same file and is encrypted too.

## security/encrypt/file-stream

CockroachDB: AES-CTR over every store file. In a page store a page
rewritten in place reuses its counter range unless the key or a per
write nonce changes.
