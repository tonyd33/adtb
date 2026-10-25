---
title: Where results are compressed
pick: one
raised-by: [surface/wire/binary]
options:
  frame:
    title: A frame flag and a codec negotiated at Hello
    links:
      - {rel: prior-art, to: prior/net-mongo-wire}
  buffers:
    title: Only Arrow bodies, per buffer, with LZ4 frame or ZSTD
    needs: [net/results/arrow]
    links:
      - {rel: prior-art, to: prior/net-flight}
  none:
    title: No protocol compression
---
MongoDB OP_COMPRESSED wraps any message (snappy, zlib, zstd).
SpacetimeDB compresses each query update with Brotli or Gzip.

## net/compression/frame

`Hello { compress: [zstd, lz4] }`; frames over a threshold set the
flag.

## net/compression/buffers

Arrow IPC body compression; one codec per batch.

## net/compression/none

Left to the network.
