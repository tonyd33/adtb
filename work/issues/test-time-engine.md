---
created-at: 2026-10-30
status: resolved
kind: feature
priority: 100
summary: most test time is spent in the engine; measure it and speed the engine up
closed: 2026-10-30
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

## 2026-10-30: one-apply not landed

Applying each operator of a migration once took top-level applications from 12 to 3 on a 3-operator migration, but suite user CPU stayed 547.65 s to 548.01 s, and 10 migration heavy tests fell only 2.4% in cycles. The memo already absorbs most replay cost; that is inferred, not profiled. The commit adds `Source::migrated` and a retry branch no test reaches, in code that decides what a migration logs, so it does not pay. Commit bd679189 stays unlanded. Revisit if a profile of real migrations shows a replay cost.

## 2026-10-30: sim-disk and os-instances

sim-disk holds allocated zeros by length. Suite user 591.2 s to 527.3 s and sys 138.6 s to 129.3 s, 2 runs each under load; minor faults did not fall, so most sys time stays unexplained. `disk.rs` still clones `durable` at a crash. `Sim::record` now states that no crash may happen while recording: `images` would panic, where the old code gave a wrong image. No digest covers a crash during an in-flight allocate. The digests in crash_images.rs pass on the unchanged simulator.

os-instances compiles the public API at `OsRuntime` once, in `adtb`. Clean build 2612.5 to 2064.0 rustc CPU-s, one run each, which is within the 20% that load alone moved this number across sessions. Stronger evidence: `OsRuntime` symbols in `update_concurrency` 1818 to 0, and rebuild after a one-line pager edit 130.5 to 109.1 CPU-s. Rebuild wall did not move, 28.9 s to 26.2 s within noise. Not tried: the same roots at `SimRuntime`, where test crates compile about 1800 instances each.

## 2026-10-30: page-copy, and closed

page-copy copies optimistic page reads by word into a per-thread buffer. `catalog` binary cycles -15.3%, 3 runs each. Suite user 586.5 s to 555.9 s, not resolved given runs of 527 to 585 s; suite sys 143.2 s to 156.1 s, not attributed, with no rise in 4 paired binaries.

Suite time in `check`, single runs under varying load: 70.6 s before these tracks, 47.2 s after. The rest of the engine cost is too spread out for another track: schema code has no item past 10% of user cycles. Held: `sim-fixture`, worth about 100 CPU-s at 3 to 8 GB and 46 s after every engine edit. Not pursued: coroutines in the simulator, which need `unsafe` and a design ruling. Build time continues on sim-instances.

## 2026-10-30: resolved
