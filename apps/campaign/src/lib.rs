//! The campaign manager's schema, its migrations and its bundles, as adtb source, and the typed
//! calls generated from them: module `schema` holds the types of release 1.3, `v1_0` and `v1_3`
//! the calls of each release's bundle, and `v1_4` the browser app's calls at the same schema.

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

/// The registered queries and transactions of release 1.0, pinned at version 1.
pub const BUNDLE_1_0: &str = include_str!("../bundles/campaign-1.0.adb");

/// The registered queries and transactions of release 1.3, pinned at version 4.
pub const BUNDLE: &str = include_str!("../bundles/campaign-1.3.adb");

/// The browser app's registered calls, pinned at the unchanged schema version 4.
pub const WEB_BUNDLE: &str = include_str!("../bundles/campaign-1.4.adb");

include!(concat!(env!("OUT_DIR"), "/campaign.rs"));

pub mod web;
