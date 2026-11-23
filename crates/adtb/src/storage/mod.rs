//! The store's pager, with the tests of it that open a database.

pub use adtb_store::storage::*;

#[cfg(test)]
mod archive_tests;
#[cfg(test)]
mod backup_tests;
#[cfg(test)]
mod checkpointer_tests;
#[cfg(test)]
mod clog_lost_tests;
#[cfg(test)]
mod commit_tests;
#[cfg(test)]
mod dump_tests;
#[cfg(test)]
mod entry_fault_tests;
#[cfg(test)]
mod lost_bitmap_tests;
#[cfg(test)]
mod open_damage_tests;
#[cfg(test)]
mod page_past_log_end_tests;
#[cfg(test)]
mod quarantine_tests;
#[cfg(test)]
mod real_file_tests;
