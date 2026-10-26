---
created-at: 2026-10-25
status: open
kind: bug
priority: 590
summary: a panic in the write agreement sweep names neither the seed nor the case
tags:
  - quality
  - sim
  - test
---

# A failure in the write agreement sweep does not name its seed or case

## Report

`check_all` (`crates/adtb-sim/tests/write.rs:899`) collects disagreements with their seed, but `engine` (`:397`) asserts `Outcome::Finished` with no message (`:411`), and every `.expect` inside `run_engine` panics through `Sim::run` with only its own text. A panic in any seed and case of `write_examples_on_several_workers_agree_for_every_seed` (3 seeds for every case) names neither. The failure summary also reports `all.len() * seeds.count()` runs (`:915`), which overstates the total under the `quick` profile, where `support::sweep` cuts the seeds.

`prep` (`:980`) decides whether a statement takes parameter `t` by matching ` t `, a trailing ` t` or `t }` in its text (`:981`), which a statement mentioning a field ending in `t` before `}` would also match.

Fix: catch the run's panic in `check_all` and record it with seed and case beside the disagreements (or pass both into `engine`'s messages); count the runs actually made; declare `t` explicitly per statement.
