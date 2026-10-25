---
title: "What retry n counts"
pick: one
options:
  n-plus-one:
    title: "At most n + 1 attempts, the first included; with no option the configured limit applies, in process as on the server"
  n-attempts:
    title: "At most n attempts in all"
---
Raised by the implementability review of docs/spec/. "Rerun up to n
times" left open whether the first attempt counts, and a block run in
process without the option had no stated limit.

## spec/txn/retry-count/n-plus-one

`retry n` allows at most n + 1 attempts, the first included. With no
option, the configured limit applies, in process as on the server.

## spec/txn/retry-count/n-attempts

`retry n` allows n attempts. Then `retry 0` runs nothing, and the word
"retry" counts the first try.
