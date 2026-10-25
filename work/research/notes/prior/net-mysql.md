---
title: MySQL client/server protocol
---
3-byte length and 1-byte sequence id per packet, sequence reset per
command; one command at a time per connection.
https://dev.mysql.com/doc/dev/mysql-server/latest/page_protocol_basic_packets.html
