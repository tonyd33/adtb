# The pager, its log and recovery

The contract with heaps, indexes, replication, backup and verify is
[storage.md](../../../../docs/design/storage.md). Its tables of damage kinds and shape codes are
the on-disk format another binary reads: change a code there and in the decoder together; the
tests of `damage_log.rs` read both tables from it.

- Every page record is logged by `Append::change_page`, reached through `Pager::apply` or
  `apply_freeing` everywhere but `quarantine.rs`; `PageExclusive::change`, `Pager::change` and
  `update_with` reach it so. A new writer of pages goes through them.
- Whether a page is allocated is read through `bitmapped` alone; `read_map` stays in `alloc.rs`.
- A frame `admit_unchained` read in while the bitmap called its page free may next change only
  by an image that starts a chain; `DirtyTable::changing` panics otherwise.
- A failed storage operation panics only through `fatal`; the row `storage_io_panics` of
  `lint/rules.toml` finds any other panic over one.
- The engine's tests that open a database sit in `crates/adtb/src/storage/`.

Tests: `cargo nextest run -p adtb-store -E 'test(/^storage::/)'`;
`cargo nextest run -p adtb --lib -E 'test(/^storage::/)'`; the scripts under
`crates/adtb-sim/scripts/faults/`. The damage search, `crates/adtb-sim/tests/storage/damage_search/`,
runs one case of each page kind and fault under `cargo xtask test --profile damage`. Under
`cargo xtask test --profile deep -E 'binary(=storage) and test(/^damage_search::/)'` it runs 32
cases of each and 4 that reach each cell, fails on a row of the mark table no case asked, and
needs two cases of each second fault. The pager crash table's cells run only under deep:
`cargo xtask test --profile deep -E 'package(=adtb-store) and test(/^storage::pager_tests::every_cell_/)'`.
