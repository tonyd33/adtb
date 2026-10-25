# Redis

kind: system
home: https://redis.io/

## RESP3

- Connections start with HELLO to negotiate the protocol version; a server not knowing HELLO speaks only RESP2. [V 2026-10-25] https://github.com/redis/redis-specifications/blob/master/protocol/RESP3.md
- Typed replies (map, set, double, boolean, null) so clients need no per-command knowledge of reply shape. Push type `>` carries out-of-band data (pub/sub, client tracking) dispatched by its first string element. Streamed strings `$?` and aggregates `*?` for unknown length. Attribute type `|` attaches metadata beside a reply. [V 2026-10-25] https://github.com/redis/redis-specifications/blob/master/protocol/RESP3.md
