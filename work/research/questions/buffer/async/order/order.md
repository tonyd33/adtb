---
title: io_uring does not order an fsync after in-flight writes
pick: one
raised-by: [buffer/async/uring-only, buffer/async/uring-fallback]
options:
  wait-cqe:
    title: Submit the fsync only after the writes it covers have completed
  link:
    title: Chain write and fsync SQEs with IOSQE_IO_LINK
  drain:
    title: Mark the fsync IOSQE_IO_DRAIN
---
`IORING_OP_FSYNC` may complete before writes submitted earlier are
issued (io_uring_enter(2)). The WAL rule (log durable before the page)
and commit acknowledgment both need order.
https://man7.org/linux/man-pages/man2/io_uring_enter.2.html

## buffer/async/order/wait-cqe

The pager tracks completions per LSN range and submits the force when
the range is complete. Explicit, one extra round of latency.

## buffer/async/order/link

`write(log blocks) -> fsync` as one chain; a failed link cancels the
rest with `-ECANCELED`. Order holds only within the chain.

## buffer/async/order/drain

The fsync waits for every earlier SQE on the ring, including unrelated
data reads; serialises the ring.
