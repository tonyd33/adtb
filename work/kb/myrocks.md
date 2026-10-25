# MyRocks

kind: system
home: https://github.com/facebook/mysql-5.6/wiki

## Key encoding

- Every key starts with a 4-byte index number, then the memcomparable form of the key columns; a secondary index key is index number, null flag, secondary key columns, primary key columns. [V 2026-10-25] https://github.com/facebook/mysql-5.6/wiki/MyRocks-record-format
- Integers: big-endian; negative values have all bits negated, positive values have the high bit set. Nullable column: one leading flag byte, 0 means NULL. [V 2026-10-25] same
- VARBINARY/VARCHAR: groups of 8 bytes each followed by one marker byte; marker 1-8 means that many significant bytes and last group, 9 means 8 significant and more follow (12.5% overhead, no escaping). Space-padding collations use chunk flags 1 (less than spaces), 2 (final), 3 (greater than spaces). [V 2026-10-25] same
- Collations whose memcomparable image loses information store restore data in the value: tag 0x02, 2-byte length, data ("unpack info"). Values may carry an 8-byte timestamp and a 9-byte checksum (tag 0x01 plus two CRC32). [V 2026-10-25] same
- Secondary index entries deleted with RocksDB SingleDelete. [U]
