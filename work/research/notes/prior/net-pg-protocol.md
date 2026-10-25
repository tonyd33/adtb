---
title: Postgres frontend/backend protocol v3 and 3.2
---
Type byte plus int32 length per message. Extended query: Parse, Bind,
Execute (with a row limit and PortalSuspended), Sync; pipelining with
skip-to-Sync after an error. Prepared statements are per session.
Cancel opens a new connection with PID and secret key (256-bit in
3.2, PG 18). NegotiateProtocolVersion answers an unsupported minor.
TLS by SSLRequest, or direct with ALPN `postgresql` (PG 17).
https://www.postgresql.org/docs/current/protocol-flow.html
