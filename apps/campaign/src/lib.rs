//! The campaign manager's schema, its migrations and its bundle, as adtb source.

pub const MODULE: &str = "campaign";

/// The schema of release 1.0 as a schema file: the fold of [`MIGRATIONS`]`[0]`.
pub const SCHEMA: &str = include_str!("../schema/campaign.adt");

/// The operator log of release 1.3: migration 0001 is release 1.0, and 0002 to 0004 are
/// releases 1.1 to 1.3.
pub const MIGRATIONS: [&str; 4] = [
    include_str!("../schema/migrations/0001.adt"),
    include_str!("../schema/migrations/0002.adt"),
    include_str!("../schema/migrations/0003.adt"),
    include_str!("../schema/migrations/0004.adt"),
];

/// The registered queries and transactions of release 1.0.
pub const BUNDLE: &str = include_str!("../bundles/campaign-1.0.adb");
