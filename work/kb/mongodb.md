# MongoDB

kind: system
home: https://www.mongodb.com/docs/

## Wire protocol

- 16-byte header: int32 messageLength, requestID, responseTo, opCode, little-endian; port 27017. OP_MSG 2013 is the current request/reply format; OP_COMPRESSED 2012 wraps others (compressorId 0 noop, 1 snappy, 2 zlib, 3 zstd); legacy opcodes removed in 5.1. [V 2026-10-25] https://www.mongodb.com/docs/manual/reference/mongodb-wire-protocol/
- OP_MSG flags: bit 0 checksumPresent (CRC-32C, used on non-TLS connections), bit 1 moreToCome (another message follows; the receiver must not reply until one without it), bit 16 exhaustAllowed (client accepts several replies). Sections: kind 0 one BSON body, kind 1 document sequence for bulk. [V 2026-10-25] https://www.mongodb.com/docs/manual/reference/mongodb-wire-protocol/
- requestID/responseTo let replies be matched to requests. [U]
