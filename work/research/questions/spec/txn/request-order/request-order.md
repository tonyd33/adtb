---
title: "Whether a client's request numbers must increase"
pick: one
options:
  increasing:
    title: "The numbers a client sends with one client id must increase"
  any-order:
    title: "No ordering requirement; numbers below the low-water mark are refused"
---
Raised by the implementability review of docs/spec/. Concurrent calls
of one client may arrive out of order. The accepted
spec/idempotency/recorded titles its design "increasing per client";
dropping the requirement is a change to that ruling and goes through
it first.

## spec/txn/request-order/increasing

As transactions §9.1 states: the numbers a client sends with one
client id must increase.

## spec/txn/request-order/any-order

Requests are deduplicated by (client id, request number), with no
ordering requirement; numbers below the client's low-water mark are
refused.
