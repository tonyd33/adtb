---
title: What the engine traces
pick: any
raised-by: [ops/expose/otel]
options:
  query-span:
    title: One span per execution, named by the registered query
    links:
      - {rel: prior-art, to: prior/ops-otel-db}
  operator-spans:
    title: Child spans for planning, each pipeline, commit and each wait
  propagate:
    title: The protocol carries the caller's trace context to the engine
    needs: [surface/wire/binary]
---
OpenTelemetry database spans name the span by `db.query.summary` and
record `db.query.text`, which is safe unsanitised when the query is
parameterised. Registered queries are parameterised by construction.

## ops/tracing/query-span

```
span "login_key"   db.system.name=adtb  db.namespace=app
                   db.query.summary=login_key  db.query.text="from i in identities ..."
                   adtb.bundle=web-2.3  adtb.arms_pruned=[Apple]
```

## ops/tracing/operator-spans

Server-side detail: plan or code-compile time, morsel pipelines,
SSI commit check, log flush wait. Costly at high rates unless sampled.

## ops/tracing/propagate

W3C `traceparent` in the Execute message, so engine spans join the
caller's trace.

```
-> Execute { query: #7f3c, bundle: web-2.3, args: [uuid], trace: "00-4bf9...-01" }
```
