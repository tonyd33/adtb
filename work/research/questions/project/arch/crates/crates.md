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
adtb/src/{lang,catalog,exec,txn,storage,sim}/  bin/{adtb,adtb-server}.rs
```

## project/arch/crates/by-layer

DataFusion and arrow-rs publish many crates (unverified count).
Compile-parallel, forced public APIs between layers.

```
adtb-lang  adtb-catalog  adtb-exec  adtb-txn  adtb-storage  adtb-server  adtb-sim
```

## project/arch/crates/by-boundary

Split where a part has different dependencies or consumers:

```
adtb-lang       -- parser, types, exhaustiveness, desugaring; no I/O; used by codegen tools
adtb-reference  -- the oracle (project/oracle); depends on adtb-lang only
adtb            -- engine core: catalog, exec, txn, storage, over an I/O trait
adtb-sim        -- simulated I/O, scheduler, fault injection; dev-dependency
adtb-server     -- wire protocol around adtb
```
