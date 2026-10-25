# Apache Arrow

kind: format
home: https://arrow.apache.org/docs/format/Columnar.html

## Unions

- Dense union: int8 types buffer plus int32 offsets, children compact. Sparse union: no offsets, every child full length, more amenable to vectorized evaluation. No top-level validity bitmap for either. [V 2026-10-25] https://arrow.apache.org/docs/format/Columnar.html
- Union types buffer is 8-bit signed type ids; more than 128 types is modeled as a union of unions. Dense union offsets are int32 and increasing per child. [V 2026-10-25] https://arrow.apache.org/docs/format/Columnar.html

## Buffers

- An array with null count 0 may omit its validity bitmap. Buffers should be 8- or 64-byte aligned and padded; 64 preferred for SIMD. [V 2026-10-25] https://arrow.apache.org/docs/format/Columnar.html
- String view: 16-byte views, strings up to 12 bytes inline; longer ones hold 4-byte length, 4-byte prefix, buffer index, offset. [V 2026-10-25] same page

## IPC

- Streaming format: Schema message, then DictionaryBatch and RecordBatch messages, optional end-of-stream marker. File format: magic ARROW1, stream, footer with schema and block offsets for random access, magic. Dense and sparse unions serialize in IPC like other nested types. [V 2026-10-25] https://arrow.apache.org/docs/format/Columnar.html

## IPC

- Stream: schema message, dictionary batches, record batches, EOS; dictionaries may interleave with batches if defined before use; delta dictionaries append. [V 2026-10-25] https://arrow.apache.org/docs/format/Columnar.html
- Encapsulated message: 0xFFFFFFFF continuation, int32 LE metadata length, Flatbuffers Message, padding to 8 bytes, body in 8-byte multiples. [V 2026-10-25] https://arrow.apache.org/docs/format/Columnar.html
- Body compression per buffer: LZ4 frame or ZSTD, one codec per batch, 8-byte uncompressed length prefix, -1 meaning uncompressed buffer. [V 2026-10-25] https://arrow.apache.org/docs/format/Columnar.html

## Flight

- RPC framework on gRPC and IPC. Methods: Handshake, ListFlights, GetFlightInfo, PollFlightInfo, GetSchema, DoGet, DoPut, DoExchange, DoAction. [V 2026-10-25] https://arrow.apache.org/docs/format/Flight.html
- FlightInfo carries schema and several FlightEndpoints, each a Ticket plus Location URIs where it can be redeemed, enabling parallel fetch from several nodes; URI schemes `grpc:`, `grpc+tls:`, `grpc+unix:`, and `arrow-flight-reuse-connection://?`. Auth: handshake tokens, header middleware, mTLS. [V 2026-10-25] https://arrow.apache.org/docs/format/Flight.html
- Flight SQL: GetFlightInfo(CommandStatementQuery) then DoGet with tickets; prepared statements via ActionCreatePreparedStatementRequest, parameters bound by DoPut; CommandStatementIngest for bulk; ActionBeginTransaction/EndTransaction, savepoints, Substrait plans; session options usually via RFC 6265 cookies. [V 2026-10-25] https://arrow.apache.org/docs/format/FlightSql.html

## ADBC

- An API standard, not a wire protocol; objects Database (shared state), Connection, Statement (one-off or prepared); results read as RecordBatchReader-like Arrow streams; `ExecutePartitions()` exposes partitioned results as in Flight. [V 2026-10-25] https://arrow.apache.org/adbc/current/format/specification.html
