---
title: What the audit records of a dump or restore passphrase
pick: one
options:
  param-only-omitted:
    title: The `encrypt` and `decrypt` operand must be a parameter, never part of the text, and a Call row's `args` hash leaves that parameter's value out
  keyed-hashes:
    title: Audit `text` and `args` become hashes keyed by a per-database secret
  as-is:
    title: Record the passphrase in `args` and `text` hashes like any other value
---
A Call row's `text` is an unkeyed BLAKE3 of ad hoc text and `args` an unkeyed BLAKE3 of the arguments. A passphrase in either is a fast hash anyone who may read `catalog::audit` can guess offline, which defeats Argon2id's cost.

## security/dump-key-audit/param-only-omitted

The text then never holds the passphrase, so its hash is safe, and `args` omits the one value. Local to two commands; the CLI's `--encrypt KEY` and `--decrypt KEY` pass it as a parameter. A text with a literal operand is refused at check.

## security/dump-key-audit/keyed-hashes

Protects every audited value, but changes every Call row, needs a secret the file keeps that a reader of the file has anyway, and loses comparison of hashes across databases.

## security/dump-key-audit/as-is

Leaves an offline guess at a fast hash of the passphrase to every reader of the audit.
