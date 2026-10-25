---
title: "The type of the password parameter of alter role"
pick: one
options:
  text:
    title: "Text"
  bytes:
    title: "Bytes"
---
Raised by the ad hoc statement checker. `alter role R password p` takes
the password as a parameter (security.md §2.1), and nothing gave its
type, so the parameter could not be checked.

## spec/password-param/text

`p` is `Text`. SCRAM normalizes a password as text (SASLprep), so a
password is text before it is anything else.

```adt fragment
-- given p: Text
alter role web password p
```

## spec/password-param/bytes

`p` is `Bytes`, passed to the verifier unchanged. Allows any byte
string, but SCRAM's normalization then has nothing to act on, and two
clients encoding one password differently derive different verifiers.
