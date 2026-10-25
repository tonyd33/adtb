---
title: SQLCipher page encryption
---
AES-256-CBC per page, a fresh random IV per page write, HMAC-SHA512
over IV and ciphertext, all stored in each page's reserved tail; key
from PBKDF2-HMAC-SHA512.
https://www.zetetic.net/sqlcipher/design/
