---
title: MongoDB wire protocol OP_MSG
---
Header with requestID and responseTo; OP_MSG flags `moreToCome` and
`exhaustAllowed` let one request get a stream of replies; document
sequences for bulk; OP_COMPRESSED with snappy, zlib or zstd.
https://www.mongodb.com/docs/manual/reference/mongodb-wire-protocol/
