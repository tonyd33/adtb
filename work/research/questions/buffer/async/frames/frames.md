---
title: The kernel holds frame memory while an I/O is in flight
pick: one
raised-by: [buffer/async/uring-only, buffer/async/uring-fallback]
options:
  pool-owned:
    title: Frames belong to the pool for the process lifetime and I/O pins a frame
  owned-buffers:
    title: Each operation takes ownership of its buffer and returns it on completion
---
With completion-based I/O the kernel writes into the buffer after
submission; in Rust a borrowed slice may be dropped or moved while the
future is cancelled.

## buffer/async/frames/pool-owned

Frames are allocated once at startup (no free), so their addresses
never dangle; a frame in I/O is pinned in its state word and eviction
skips it. Cancellation leaves the frame pinned until the CQE arrives.

```
frame state: ... | io_in_progress | pin count
submit: pin(frame); sqe.addr = frame.ptr
cqe:    unpin(frame); publish or mark error
```

## buffer/async/frames/owned-buffers

tokio-uring and monoio: `file.read_at(buf, off).await` returns
`(result, buf)`. Natural with heap buffers, awkward for a fixed frame
array. https://github.com/tokio-rs/tokio-uring
