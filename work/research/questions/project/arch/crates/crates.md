---
title: How the implementation is split into crates
pick: one
raised-by: [substrate/new-engine]
options:
  single:
    title: One library crate with modules, plus thin binaries
    links:
      - {rel: prior-art, to: prior/buffer-tigerbeetle}
  by-layer:
    title: A crate per layer (catalog, storage, txn, exec, lang, server)
    links:
      - {rel: prior-art, to: prior/exec-datafusion}
  by-boundary:
    title: Crates only at stability or dependency boundaries
---
The repository is one binary crate today (`src/main.rs`). Layers
named by settled rulings: language front end and type checker, catalog
as relations, planner and executor, transaction manager with SSI,
storage (pager, heaps, union index, log), simulator, server.

## project/arch/crates/single

TigerBeetle is one source tree built by one toolchain. Modules give
privacy; one compilation unit lets the optimizer inline across layers.

```
adtdb/src/{lang,catalog,exec,txn,storage,sim}/  bin/{adtdb,adtdb-server}.rs
```

## project/arch/crates/by-layer

DataFusion and arrow-rs publish many crates (unverified count).
Compile-parallel, forced public APIs between layers.

```
adtdb-lang  adtdb-catalog  adtdb-exec  adtdb-txn  adtdb-storage  adtdb-server  adtdb-sim
```

## project/arch/crates/by-boundary

Split where a part has different dependencies or consumers:

```
adtdb-lang       -- parser, types, exhaustiveness, desugaring; no I/O; used by codegen tools
adtdb-reference  -- the oracle (project/oracle); depends on adtdb-lang only
adtdb            -- engine core: catalog, exec, txn, storage, over an I/O trait
adtdb-sim        -- simulated I/O, scheduler, fault injection; dev-dependency
adtdb-server     -- wire protocol around adtdb
```
