---
title: Key hierarchy and rotation
pick: one
raised-by: [security/encrypt/page, security/encrypt/file-stream]
options:
  wrapped:
    title: A key-encryption key wraps data keys; pages name their data key version and rotate lazily as rewritten
    links:
      - {rel: prior-art, to: prior/security-cockroach-ear}
      - {rel: prior-art, to: prior/security-pg-tde}
  single:
    title: One key; rotation rewrites the whole file
    links:
      - {rel: prior-art, to: prior/security-sqlcipher}
---
Rotation has two parts: replacing the outer key (cheap with wrapping)
and retiring an exposed data key (every page under it must be
rewritten).

## security/keys/wrapped

```
$ adt key rotate --kek kms://vault/adt-app/v3     -- rewraps data keys
$ adt key rotate --dek                            -- new pages use dek 8
```

Cockroach rotates data keys and relies on compaction to rewrite old
files; a page store has no compaction, so retiring a data key needs a
background rewrite pass over the pages still under it, found by the key
version in each page header.

## security/keys/single

Simple; retiring the key is a full copy.
