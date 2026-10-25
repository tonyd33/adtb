---
title: "What a restore with the wrong passphrase reports"
pick: one
options:
  as-damaged:
    title: "The same failure as a damaged dump"
  wrong-key:
    title: "A distinct wrong-key failure, from a check value in the manifest"
---
Raised by the report on `spec/dump-key/passphrase`. An encrypted dump
is sealed with authenticated encryption under a key derived from the
passphrase.

## spec/dump-wrong-key/as-damaged

A restore with the wrong passphrase fails as a restore of a damaged
dump does. Authenticated encryption cannot tell a wrong key from
damaged ciphertext: both fail the tag, and the spec says so.

## spec/dump-wrong-key/wrong-key

The manifest also holds a check value sealed under the key, as a
database file's header does, and a wrong passphrase is reported apart.
Needs one more manifest field, and a check value lets an attacker test
passphrases offline against it, as the tag already does.
