---
title: How operators reach the engine
pick: some
raised-by: [substrate/new-engine]
options:
  cli:
    title: One command-line tool that works on a closed file and on a running server
  in-language:
    title: Operations are expressible in the query language, by a privileged role
  console:
    title: A web console served by the server
    links:
      - {rel: prior-art, to: prior/ops-crdb}
---
Embedded and server share a core (settled `engine/embedded-server`).
For an embedded file there is no server to connect to; the operator
has the file.

## ops/admin/cli

```
$ adt check app.db
$ adt compact app.db --into app2.db
$ adt explain app.db queries.login_key
$ adt repl app.db --role admin
$ adt upgrade app.db --finalize
```

Opening the file takes the write lock embedded writers also take.

## ops/admin/in-language

```
from s in sys::sessions select { id: s.id, state: s.state }
vacuum identities
cancel sys::sessions#4471
```

## ops/admin/console

CockroachDB DB Console: dashboards, statement and contention pages.
Server only.
