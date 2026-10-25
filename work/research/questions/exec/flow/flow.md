---
title: Which way data flows between operators
pick: one
raised-by: [substrate/new-engine]
options:
  pull:
    title: Consumers pull batches from producers
    links:
      - {rel: prior-art, to: prior/exec-datafusion}
      - {rel: prior-art, to: prior/exec-x100}
  push:
    title: Producers push into consumers, plans split into pipelines
    links:
      - {rel: prior-art, to: prior/exec-hyper}
---
A lowered match is a union of arm queries. Under pull the union is an
operator that drains children; under push each arm is its own pipeline
whose sink is shared.

## exec/flow/pull

```
trait Exec { fn poll_next(&mut self, cx: &mut Context) -> Poll<Option<Batch>>; }
```

DataFusion `SendableRecordBatchStream` (async, unverified details);
X100 and early DuckDB pulled vectors.

## exec/flow/push

```
trait Sink { fn consume(&mut self, b: &Batch); fn finish(&mut self); }
```

HyPer, Umbra; DuckDB switched to push in 2021 (unverified date). An
arm pipeline is `scan(heap_c) -> arm_c -> sink`; the union disappears
into the sink.
