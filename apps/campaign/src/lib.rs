//! Exercise adtb through the campaign schema, migrations, typed bundles and web server.

pub const MODULE: &str = "campaign";

/// The schema of release 1.0 as a schema file: the fold of [`MIGRATIONS`]`[0]`.
pub const SCHEMA: &str = include_str!("../schema/campaign.adt");

/// The operator log through the prep application's same-campaign reference checks.
pub const MIGRATIONS: [&str; 5] = [
    include_str!("../schema/migrations/0001.adt"),
    include_str!("../schema/migrations/0002.adt"),
    include_str!("../schema/migrations/0003.adt"),
    include_str!("../schema/migrations/0004.adt"),
    include_str!("../schema/migrations/0005.adt"),
];

/// The registered queries and transactions of release 1.0, pinned at version 1.
pub const BUNDLE_1_0: &str = include_str!("../bundles/campaign-1.0.adb");

/// The registered queries and transactions of release 1.3, pinned at version 4.
pub const BUNDLE: &str = include_str!("../bundles/campaign-1.3.adb");

/// The browser app's registered calls, pinned at schema version 5.
pub const WEB_BUNDLE: &str = include_str!("../bundles/campaign-1.5.adb");

include!(concat!(env!("OUT_DIR"), "/campaign.rs"));

pub mod web;
