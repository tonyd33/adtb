---
title: Where the outer key comes from
pick: some
raised-by: [security/encrypt/page, security/encrypt/file-stream]
options:
  passphrase:
    title: Derived from a passphrase by a slow KDF
    links:
      - {rel: prior-art, to: prior/security-sqlcipher}
  kms:
    title: Fetched or unwrapped by an external KMS
    links:
      - {rel: prior-art, to: prior/security-pg-tde}
  host:
    title: Bytes passed in by the embedding application
---
The embedded core and the server have different key holders
(`engine/embedded-server`).

## security/kek/passphrase

SQLCipher: PBKDF2-HMAC-SHA512 with a salt in the file's first bytes.

## security/kek/kms

pg_tde providers: KMIP, Vault, OpenBao, and others.

## security/kek/host

```
adtb::open_with("app.db", Key::bytes(platform_keystore.get("adt")?))
```

Mobile keystores; the engine never sees where the key lives.
