//! Open a database, or the pager of a component world, on the simulator from a test.

use adtb::{CreateOptions, Database, Log, OpenError, OpenOptions};
use adtb_sim::SimRuntime;

/// The data file a test's database lives in unless the test names another.
pub const FILE: &str = "app.adt";

/// Returns the options that open the database with `workers` workers, creating it if it does not
/// exist.
pub fn options(workers: usize) -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(workers),
        ..OpenOptions::default()
    }
}

/// Open the database in `file` with the schema `log` and `options`.
pub fn open(
    rt: &SimRuntime,
    file: &str,
    log: &Log,
    options: &OpenOptions,
) -> Result<Database<SimRuntime>, OpenError> {
    Database::open_in(rt.clone(), file, log, options)
}

/// Open the database as [`open`] does. Panics, naming `file`, when it fails.
pub fn opened(
    rt: &SimRuntime,
    file: &str,
    log: &Log,
    options: &OpenOptions,
) -> Database<SimRuntime> {
    open(rt, file, log, options).unwrap_or_else(|e| panic!("{file} opens: {e:?}"))
}
