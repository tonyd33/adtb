---
title: How the server schedules connections
pick: one
raised-by: [engine/embedded-server]
options:
  thread:
    title: A thread or process per connection
    assessment: {grade: costly, why: idle connections cost a thread each; the reason PgBouncer exists}
  async:
    title: An async I/O front end; calls become tasks on the morsel scheduler
    links:
      - {rel: prior-art, to: prior/net-tigerbeetle-client}
---
Execution is morsel-driven (`exec/parallel/morsel`), so worker threads
already belong to the scheduler, not to sessions.

## net/server/thread

Postgres backend per connection; MySQL thread per connection.

## net/server/async

A few I/O threads parse frames and enqueue calls; an idle connection
is a buffer and a socket. Supavisor holds 250k idle connections per
node this way (README claim).
