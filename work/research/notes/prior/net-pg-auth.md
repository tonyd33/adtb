---
title: Postgres SCRAM-SHA-256 and OAuth
---
SASL exchange: SCRAM-SHA-256, or SCRAM-SHA-256-PLUS with
`tls-server-end-point` channel binding. PG 18 adds an `oauth` method
with validator libraries and deprecates MD5.
https://www.postgresql.org/docs/current/sasl-authentication.html
