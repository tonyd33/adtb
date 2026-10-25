# SQLCipher

kind: system
home: https://www.zetetic.net/sqlcipher/

## Encryption

- AES-256-CBC per page; a random IV per page regenerated on every write, stored at the end of the page; HMAC-SHA512 over ciphertext and IV per page; PBKDF2-HMAC-SHA512 key derivation, 256,000 iterations by default (`PRAGMA kdf_iter`); first 16 bytes of the file are a random salt. [V 2026-10-25] https://www.zetetic.net/sqlcipher/design/
