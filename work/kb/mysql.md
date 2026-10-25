# MySQL

kind: system
home: https://dev.mysql.com/doc/

## Wire protocol

- Packet: 3-byte payload length, 1-byte sequence id; max payload 2^24-1, larger payloads split; sequence id resets to 0 at each new command. [V 2026-10-25] https://dev.mysql.com/doc/dev/mysql-server/latest/page_protocol_basic_packets.html
- One command at a time per connection (no pipelining in the base protocol). [S 2026-10-25] same page, summary only
