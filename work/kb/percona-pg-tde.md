# Percona pg_tde

kind: system
home: https://docs.percona.com/pg-tde/

## Encryption

- Ships with Percona Server for PostgreSQL and needs its patches. Key providers: KMIP, Vault, OpenBao, Fortanix, Thales, Akeyless, keyring file. [V 2026-10-25] https://docs.percona.com/pg-tde/index.html
- Data files and indexes AES-128-CBC, WAL AES-128-CTR, under internal keys, one per relation; internal keys AES-128-GCM under one principal key per database held in a KMS. Encrypted: data files, indexes, sequences, temp tables, WAL. Not encrypted: system catalogs, statistics, server logs. Principal key rotation re-encrypts internal keys only. [V 2026-10-25] https://docs.percona.com/pg-tde/architecture/index.html
