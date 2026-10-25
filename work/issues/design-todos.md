---
created-at: 2026-10-25
status: open
kind: feature
priority: 900
summary: "docs/design/ carries 18 TODO lines for undecided interfaces, layouts and policies"
tags: [design]
---

# Feature: settle the TODOs in docs/design

## Report

`grep -rn TODO docs/design/` on 2026-10-25:

- `server.md:12`: the interface by which the server submits a call to
  `adtb` and receives its batches is undecided.
- `server.md:39`: how the server binds a connection to a bundle through
  `adtb` (spec/wire.md §3.4) is undecided.
- `kernel.md:25`: whether `adtb-lang` or the engine performs
  normalisation is undecided. Ruled since: `adtb-lang` owns it
  (`kernel-normalisation-and-analyses.md`).
- `kernel.md:27`: the remaining invariants of a kernel term (constructor
  sets listed at each source, fills explicit) and its representation
  are undecided.
- `kernel.md:37`: the API by which `adtb-lang` gives a code generator
  these ids and ordinals is undecided.
- `mvcc.md:29`: the structure of a heap's page directory (a page list
  or a tree) is undecided.
- `mvcc.md:308`: the SIREAD lock a hash index probe takes is undecided;
  the candidate is the probed hash value and constructor set, as an
  equality range.
- `storage.md:56`: the byte layout of a header copy is undecided.
- `storage.md:86`: the offsets of `kind`, `owner` and `flags` are
  undecided; only the LSN, the 32-bit checksum and the per-page format
  version are fixed by rulings.
- `storage.md:434`: the replacement policy is undecided; the marked
  state of the page state word supports a clock with a one-bit
  reference count.
- `execution.md:121`: the identity by which a plan and an expression
  index name the code of a user-defined function is undecided.
- `execution.md:148`: the code generator backend is deferred.
- `oracle.md:55`: the interface a generator exposes to the harness is
  undecided.
- `runtime.md:140`: closing a file, file sizes other than at `Open`,
  and renames are undecided.
- `schema-change.md:75`: the contract of the recheck pipeline for
  registered queries is undecided.
- `indexes.md:187`: whether a hash index is chosen by the engine or
  declared by the schema is undecided.
- `indexes.md:269`: split point choice and key compression are
  deferred.
- `sys.md:35`: what each of `Frame` and `PageRef` represents, and its
  methods, are undecided.

Each is settled when its milestone starts, through the design space
where it needs a ruling.

## 2026-10-25: owners

`grep -rn TODO docs/design/` now finds 18 lines: the two at
`kernel.md:25` and `kernel.md:27` are gone, `kernel.md:141` and
`kernel.md:143` are new, and some lines moved. Each is settled by the
ticket of the milestone that needs it:

- `storage.md:56`, `storage.md:86`, `sys.md:35`, `runtime.md:140`:
  `page-buffer-contract.md`;
- `storage.md:434`: `vmcache.md`;
- `mvcc.md:29`: `slotted-heap.md`;
- `indexes.md:187`: `blink-tree.md`; `indexes.md:269` stays deferred;
- `mvcc.md:308`: `ssi.md`;
- `execution.md:121`: `executor.md`; `execution.md:148` stays deferred;
- `oracle.md:79`: `engine-differential-tests.md`;
- `schema-change.md:75`: `bundles.md`;
- `kernel.md:398`: `codegen.md`;
- `kernel.md:141`: `spec-field-eval-order.md`;
- `kernel.md:143`: `anon-sum-ctor-index.md`;
- `server.md:12`, `server.md:39`: milestone 10, not yet ticketed.
