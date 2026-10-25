---
title: How page encryption meets the page checksum
pick: one
raised-by: [security/encrypt/page]
options:
  aead:
    title: An authenticated cipher's tag replaces the checksum; a failed tag is a corrupt or forged page
    needs: [buffer/faults/corrupt-detect]
  inner:
    title: The checksum is over plaintext and encrypted with it; checking needs the key
  outer:
    title: The checksum is over ciphertext; tools check pages without the key
---
`buffer/faults/corrupt-detect` promises detection of corrupted pages.
An unauthenticated cipher lets an attacker flip ciphertext bits
undetected by anything but a plaintext checksum.

## security/integrity/aead

AES-GCM's 16-byte tag in the page trailer (unverified size for this
layout); SQLCipher's HMAC-SHA512 per page. Detects tampering, not only
corruption; a page swapped for an older valid version of itself is
caught only by the LSN check (`buffer/faults/stale-page/lsn-check`)
when the tag binds page id and LSN.

## security/integrity/inner

Postgres TDE's open question: CRC tools would need the key.

## security/integrity/outer

The 32-bit checksum stays as is; integrity against an attacker is not
claimed.
