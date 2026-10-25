---
title: Encryption costs a copy per page I/O
pick: any
raised-by: [security/encrypt/page]
options:
  staging:
    title: Encrypt into a per-I/O staging buffer on write; decrypt in place in the frame on read
  live:
    title: Live with it
    excludes: [security/encrypt/io-cost/staging]
---
With direct I/O the page frame is the I/O buffer. Encrypting in place
would hide the page from concurrent optimistic readers
(`btree/latch/olc`) while it is written.

## security/encrypt/io-cost/staging

One extra buffer per in-flight write; reads decrypt after completion,
before the frame is published.

## security/encrypt/io-cost/live

Every write locks the frame against readers for its duration.
