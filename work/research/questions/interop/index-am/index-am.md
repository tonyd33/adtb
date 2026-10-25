---
title: Whether new index kinds can be added outside the engine
pick: some
options:
  fixed:
    title: The engine ships a fixed set of index kinds
    excludes: [interop/index-am/am-trait, interop/index-am/framework]
  am-trait:
    title: An access-method trait with build, insert, scan, vacuum and cost callbacks
    links:
      - {rel: prior-art, to: prior/interop-pg-indexam}
  framework:
    title: A generalized search tree where the extension supplies only key semantics
    links:
      - {rel: prior-art, to: prior/interop-pg-indexam}
  derived:
    title: A special index is a B-tree over a relation derived from the indexed one by a function
---
Every index ranges over a constructor set (`index/ctor-set`) and
entries name their constructor (`btree/entry-order/after-key`).
Full-text, spatial and vector search each want an index that is not
a B-tree over the field.

## interop/index-am/fixed

B-trees (`btree/structure/blink`), plus whatever `interop/fts`,
`interop/spatial` and `interop/vector` put in core.

## interop/index-am/am-trait

```rust
trait IndexKind {
  fn build(&self, heaps: &[HeapScan], out: &mut PageWriter) -> Result<()>;
  fn insert(&self, key: &Value, ctor: CtorId, tid: Tid, tx: &Tx) -> Result<()>;
  fn scan(&self, pred: &Pred, tx: &Tx) -> Box<dyn Iterator<Item = (CtorId, Tid)>>;
  fn bulk_delete(&self, dead: &DeadSet) -> Result<()>;
  fn cost(&self, pred: &Pred, stats: &Stats) -> Cost;
}
```

Postgres `IndexAmRoutine`; the core knows nothing of an index beyond
it (https://www.postgresql.org/docs/current/indexam.html).

## interop/index-am/framework

```rust
trait GistKey { fn consistent(..); fn union(..); fn penalty(..); fn pick_split(..); }
```

GiST: "the GiST layer itself takes care of concurrency, logging and
searching the tree structure"
(https://www.postgresql.org/docs/current/gist.html). R-trees,
range types and exclusion over ranges are instances.

## interop/index-am/derived

```
add text index (body) in docs.(Article | Note) tokenize english
-- maintains relation docs_terms : { term: Text, doc: Key docs, pos: I32 }
-- with a B-tree on (term, doc)
```

The derived relation is maintained from each change set's delta like a
constraint check (`constraint/enforce/delta`); no new page format, WAL
record or lock kind. Covers inverted and geohash-style indexes, not
graph indexes.
