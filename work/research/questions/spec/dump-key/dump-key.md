---
title: "The type of the key of dump ... encrypt"
pick: one
options:
  passphrase:
    title: "A Text passphrase, turned into the key by Argon2id with a salt and parameters the manifest records"
  raw:
    title: "Bytes of a 256-bit key"
  source:
    title: "A sum of the key sources of a database file: passphrase, KMS key or host bytes"
---
Raised by the ad hoc statement checker. `dump to "path" encrypt k`
takes the key as a parameter (syntax.md §12, interop.md §6.1), and
nothing gave its type. security.md §9.3 gives the key material of a
database file: a KEK from a passphrase, a KMS or host bytes, wrapping
256-bit DEKs.

## spec/dump-key/passphrase

`k` is `Text`, a passphrase. The dump's key is derived from it by
Argon2id, as a passphrase KEK is, with a salt and parameters recorded
in the manifest, so a restore needs only the passphrase.

```adt fragment
-- given k: Text
dump to "app.dump" encrypt k
```

A dump travels between machines; a passphrase is the one key source
that needs nothing on the restoring side but the person holding it.

## spec/dump-key/raw

`k` is `Bytes`, the 256-bit key itself. No derivation, but a value of
another length needs its own refusal, and a caller holding only a
passphrase must run a KDF itself.

## spec/dump-key/source

`k` is a prelude sum, `Passphrase { text: Text } | Kms { uri: Text } |
Host { key: Bytes }`, mirroring a database file's KEK sources. The
most general, but adds a prelude sum and a KMS dependency to restore.
