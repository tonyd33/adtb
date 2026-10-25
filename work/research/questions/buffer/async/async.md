---
title: The interface the pager issues I/O through
pick: one
raised-by: [engine/own-pager]
options:
  blocking-pool:
    title: Blocking pread and pwrite on a pool of I/O threads
    links:
      - {rel: prior-art, to: prior/buffer-pg}
  uring-only:
    title: io_uring only
    excludes: [buffer/platform/portable]
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
  uring-fallback:
    title: io_uring on Linux behind an interface with portable backends
---
A B-tree leaf scan or a merge-append sum scan wants many outstanding
reads; a checkpoint wants many outstanding writes.

## buffer/async/blocking-pool

Postgres `io_method=worker` (18): backends hand reads to worker
processes. Portable; one context switch per I/O; depth equals
threads.
https://www.postgresql.org/docs/current/runtime-config-resource.html

## buffer/async/uring-only

TigerBeetle: submission and completion rings shared with the kernel
(Linux 5.1+), `O_DIRECT`, no thread pool. Some hosts disable io_uring
for sandboxing (Google kCTF note in TigerBeetle's safety doc).
Rust: glommio (thread-per-core, Linux only).
https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/concepts/safety.md

```
sqe: { op: READ, fd, off: pid * 4096, addr: frame, len: 4096, user_data: pid }
cqe: { user_data: pid, res: 4096 | -EIO }
```

## buffer/async/uring-fallback

Postgres `io_method=io_uring` beside `worker`; monoio (io_uring or
epoll on Linux, kqueue on macOS). The pager sees one submit/complete
interface.
https://github.com/bytedance/monoio
