---
title: Arrow Flight, Flight SQL and ADBC
---
Flight: gRPC methods (GetFlightInfo, DoGet, DoPut, DoExchange, DoAction)
carrying Arrow IPC streams; FlightInfo lists endpoints whose tickets
may be redeemed at several locations in parallel. Flight SQL adds
statements, prepared statements, transactions and ingest. ADBC is a
client API over Arrow streams, not a wire protocol.
https://arrow.apache.org/docs/format/Flight.html
https://arrow.apache.org/docs/format/FlightSql.html
