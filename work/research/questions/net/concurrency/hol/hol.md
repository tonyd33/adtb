---
title: One slow stream blocks the others on a TCP connection
pick: any
raised-by: [net/concurrency/multiplexed]
options:
  quic:
    title: Carry streams on QUIC so loss on one does not stall the rest
    needs: [net/transport/quic]
  chunked:
    title: Bound frame size and interleave chunks fairly across streams
  live:
    title: Live with head-of-line blocking; open more connections when it matters
    excludes: [net/concurrency/hol/quic, net/concurrency/hol/chunked]
---
A large result stream holds the socket; a short call behind it waits.
TCP loss stalls every stream.

## net/concurrency/hol/quic

RFC 9000 streams are independent.

## net/concurrency/hol/chunked

HTTP/2 style: batches are split to at most N bytes and the writer
round-robins streams; per-stream credit (`net/flow`) keeps a slow
reader from filling the socket.

## net/concurrency/hol/live

Clients keep a small pool.
