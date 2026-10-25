# QUIC (RFC 9000)

kind: format
home: https://www.rfc-editor.org/rfc/rfc9000.html

## Streams

- Bidirectional and unidirectional streams; client-initiated ids even, server-initiated odd (2.1). Flow control per stream (MAX_STREAM_DATA) and per connection (MAX_DATA), stream count limits (MAX_STREAMS) (4, 19.9-19.11). Streams are independent, so loss on one does not block others. [V 2026-10-25] https://www.rfc-editor.org/rfc/rfc9000.html
- 0-RTT data has no replay protection; connection migration by connection ids (9). [V 2026-10-25] https://www.rfc-editor.org/rfc/rfc9000.html
