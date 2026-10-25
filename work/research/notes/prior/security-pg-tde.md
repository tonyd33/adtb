---
title: Postgres TDE (upstream proposals and Percona pg_tde)
---
Upstream TDE is unmerged; its design hit nonce reuse from hint bits and
the checksum question. Percona pg_tde (patched server): per-relation
internal keys under a KMS principal key, CBC for data, CTR for WAL;
catalogs and statistics stay plaintext; principal rotation rewraps.
https://wiki.postgresql.org/wiki/Transparent_Data_Encryption
https://docs.percona.com/pg-tde/architecture/index.html
