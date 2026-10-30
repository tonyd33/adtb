---
created-at: 2026-10-30
status: in-progress
kind: feature
priority: 100
summary: most test time is spent in the engine; measure it and speed the engine up
branch: test-speed
---

# Feature: Test time engine

## Request

## Proposal

## Open questions

## 2026-10-30: scope

Tony: the next work is speeding up tests, engine speedups before the test framework or test
discipline, since an engine speedup speeds up all future work. Profile the suite, then fix the
largest engine costs. Correctness failures preempt tuning. engine-open-and-storage-perf and
campaign-engine-tests-slow are candidates, not assumed causes.

## 2026-10-30: verify's sweeps

Online verify added sweeps to the default run: about 15 s for the vacuum phase, about 10 s each for two concurrent-writer tests and the expression index test, about 3 s each for crash and repeated runs. Count these in the profile.

## 2026-10-30: profile

Engine 75% of suite CPU, simulator 11% of user cycles, test framework and testkit about 2.5%. Schema build and check are 44% of user cycles, `build_ahead` 30%. A prototype of a memo on `Source::built` and a faster `Vm::marked` took user CPU from 808 s to about 590 s with every test passing. Tracks, in order: schema-memo and catalog-insert now, then `Schema` behind an `Arc`, one application per migration, hardware CRC, and a shared simulator fixture sized by a new measurement. Cut no sweeps: nothing measured what a seed finds.

## 2026-10-30: catalog-insert not landed

Indexing catalog indexes by relation and comparing diffs in blocks gave -1.3% instructions and -5.3% of `fill` on the `catalog` binary, against a target of half. The relation scan was under 1% of `fill`, whose cost is spread: heap insert 31%, B-tree insert 24%, `make_room` and eviction 11%, `rows::objects` 9.5%, `build_ahead` 7.6%. That does not pay for a new type, so the commit stays unlanded. Batching catalog rows is ruled out for now: it changes log bytes and crash points for an unmeasured gain. Profile `fill` again after schema-memo.

Candidate, unmeasured as a fix: `PageOptimistic::read` copies the whole page into a fresh `Vec` at each B-tree level. `observe` is about 27% of `Catalog::insert`.

## 2026-10-30: schema-memo

Memo owned by `Source`, 32 entries, compared by full equality. Suite user CPU 798 s to 607 s, wall 86 s to 65 s, one run each under load. A process-global memo would leave 7.1% of builds rather than 13.0%, an estimated 2% of cycles; not worth process state. Review: no blockers. Check: 2977 passed.

## 2026-10-30: schema-arc

`Schema` now holds its declarations in a shared `defs: Arc<SchemaDefs>`, written through `Arc::make_mut`; `ids`, `stages` and `value_stages` stay owned. `Arc<Schema>` from `built` alone gave -4.2% cycles because `schema()` still copied for `annotate`. Suite user CPU 649.7 s to 572.5 s, one run each; sys 135.7 s to 160.9 s, not attributed. `DerefMut` stays: no site writes on a read path, and a test fails if `schema()` copies the declarations.
