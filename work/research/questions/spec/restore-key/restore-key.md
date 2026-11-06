---
title: How a restore takes the passphrase of an encrypted dump, and what it does without one or with one it does not need
pick: one
options:
  decrypt-clause-invalid:
    title: "`restore from s to p decrypt k` and `adtb restore --decrypt KEY`; an encrypted dump with no passphrase, and a plaintext dump given one, both refuse with `Invalid` before writing anything"
  key-required-reason:
    title: The same clause, with a new reject reason `KeyRequired` for an encrypted dump restored without a passphrase
  ignore-unneeded:
    title: The same clause; a passphrase given for a plaintext dump is ignored
---
Interop 6.1 and security 9.5 say a restore of an encrypted dump needs the passphrase but name no surface for it, and say nothing of a restore without one or of a passphrase given for a plaintext dump. A wrong passphrase is already ruled damage (spec/dump-wrong-key/as-damaged).

## spec/restore-key/decrypt-clause-invalid

Mirrors `dump ... encrypt k` and `--encrypt KEY`. The manifest's plaintext version fields say whether the dump is encrypted, so both mismatches are known before any entry is read and are the caller's error, not damage: nothing failed to authenticate. Refusing a passphrase for a plaintext dump keeps a forged plaintext dump swapped in for an encrypted one from restoring unchecked under a caller who expected authenticated input. Adds no reason code.

## spec/restore-key/key-required-reason

Lets a client branch on the missing key without reading a message, at the cost of a reason code for one case; `KeyRequired` exists only on `OpenError`, for files.

## spec/restore-key/ignore-unneeded

Lets a script pass a passphrase always, but restores a swapped plaintext dump unchecked for the caller who relied on encryption to authenticate it.
