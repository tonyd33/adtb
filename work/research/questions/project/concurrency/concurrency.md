---
title: Concurrent clients the engine is built for
pick: one
raised-by: [substrate/new-engine]
options:
  app-threads:
    title: Tens of threads of one application
  many-sessions:
    title: Thousands of sessions over the wire
    links:
      - {rel: gains, to: surface/wire/binary}
---
Decides whether sessions are threads, tasks or something multiplexed,
and how many SSI read locks and snapshots live at once.

## project/concurrency/app-threads

The embedding application's own threads call the in-process API.
A connection is a handle, not a server resource.

```
let tx = db.begin();
tx.run(registered::retag_to_google, (id, sub))?;
tx.commit()?;   // may return SerializationFailure; the caller retries
```

## project/concurrency/many-sessions

Postgres pools connections because each is a process; a task-per-
session server multiplexes them. SSI lock tables and the snapshot
watermark (`mvcc/watermark`) scale with live transactions.
