---
title: How a running query or session is stopped
pick: some
raised-by: [substrate/new-engine]
options:
  cancel:
    title: Cancel the running call; the transaction stays open and may roll back
    links:
      - {rel: prior-art, to: prior/ops-pg-limits}
  terminate:
    title: End the session and abort its transaction
    links:
      - {rel: prior-art, to: prior/ops-pg-limits}
  token:
    title: The embedded API takes a cancellation token per call
    needs: [surface/wire/embedded]
---
Cancellation is checked at morsel boundaries (settled `exec/parallel/morsel`),
so latency to stop is one morsel per worker.

## ops/cancel/cancel

`pg_cancel_backend`; the protocol's cancel request with a secret key.

```
cancel sys::sessions#4471
```

## ops/cancel/terminate

`pg_terminate_backend`.

## ops/cancel/token

```rust
let tok = CancelToken::new();
db.run_with(queries::report, m, &tok)?;   // tok.cancel() from another thread
```
