/// An engine-defined type's module, name and id, with each constructor's name and id.
pub type FixedType = (
    &'static str,
    &'static str,
    u64,
    &'static [(&'static str, u64)],
);

/// The fixed ids of the engine-defined types and their constructors, by module and name.
pub const TYPES: &[FixedType] = &[
    ("prelude", "Bool", 0, &[("False", 1), ("True", 2)]),
    ("prelude", "Option", 3, &[("None", 4), ("Some", 5)]),
    ("prelude", "Result", 6, &[("Ok", 7), ("Err", 8)]),
    ("prelude", "Tag", 9, &[]),
    (
        "prelude",
        "Rounding",
        10,
        &[
            ("HalfEven", 11),
            ("HalfUp", 12),
            ("HalfDown", 13),
            ("Up", 14),
            ("Down", 15),
            ("Ceiling", 16),
            ("Floor", 17),
        ],
    ),
    ("prelude", "Overflow", 18, &[("Clamp", 19), ("Reject", 20)]),
    (
        "prelude",
        "Fault",
        21,
        &[
            ("DivisionByZero", 22),
            ("ArithmeticOverflow", 23),
            ("OutOfRange", 24),
            ("NotFinite", 25),
            ("NoSuchDate", 26),
            ("UnknownZone", 27),
            ("InvalidText", 28),
            ("InvalidSpan", 29),
            ("NegativeCount", 30),
            ("NoSuchRow", 31),
            ("FunctionFailed", 32),
        ],
    ),
    ("prelude", "Uuid", 33, &[("Uuid", 34)]),
    ("prelude", "Span", 35, &[("Span", 36)]),
    (
        "prelude",
        crate::schema::prelude::PRINCIPAL,
        37,
        &[("EndUser", 38), ("Service", 39), ("Anonymous", 40)],
    ),
    (
        "prelude",
        "Origin",
        41,
        &[("Registered", 42), ("Function", 43), ("AdHoc", 44)],
    ),
    (
        "prelude",
        "Outcome",
        45,
        &[("Succeeded", 46), ("Failed", 47)],
    ),
    (
        "prelude",
        "Error",
        48,
        &[("Retry", 49), ("Rejected", 50), ("Fatal", 51)],
    ),
    ("prelude", "TxnId", 52, &[("TxnId", 53)]),
    ("prelude", "Report", 54, &[("Report", 55)]),
    ("prelude", "Position", 56, &[("Position", 57)]),
    (
        "prelude",
        "RetryReason",
        58,
        &[
            ("Serialization", 59),
            ("SafePointTimeout", 60),
            ("Overloaded", 61),
            ("StreamIdle", 62),
            ("FileChanged", 63),
            ("Fenced", 64),
            ("ReplayConflict", 341),
        ],
    ),
    (
        "prelude",
        "RejectReason",
        65,
        &[
            ("Violated", 66),
            ("KeyViolation", 67),
            ("KeyChanged", 68),
            ("NotOpen", 69),
            ("UserError", 70),
            ("Faulted", 71),
            ("Denied", 72),
            ("Invalid", 73),
            ("SchemaRefused", 74),
            ("Untranslatable", 75),
            ("BadInput", 76),
            ("SchemaMismatch", 77),
            ("Cancelled", 78),
            ("Terminated", 79),
            ("DeadlineExceeded", 80),
            ("StatementLimit", 81),
            ("TransactionExpired", 82),
            ("TransactionBusy", 83),
            ("UnknownQuery", 84),
            ("UnknownTransaction", 85),
            ("UnknownBundle", 86),
            ("VersionMismatch", 87),
            ("Retired", 88),
            ("BundleExpired", 89),
            ("BundleStale", 90),
            ("BundleSuperseded", 91),
            ("DowngradeRefused", 92),
            ("TooOld", 93),
            ("HistoryUnavailable", 94),
            ("PositionNotOnHistory", 95),
            ("WrongVersion", 96),
            ("ReadOnlyReplica", 97),
            ("RequestExpired", 98),
            ("RequestReused", 99),
            ("AuthFailed", 100),
            ("UnsupportedProtocol", 101),
            ("UnknownDatabase", 102),
            ("CatalogMismatch", 342),
            ("EditionLocked", 343),
            ("UnknownEdition", 344),
            ("MixedVersions", 345),
            ("MigrationInProgress", 372),
            ("DumpDamaged", 460),
            ("BackupDamaged", 474),
            ("RestorePointUnavailable", 475),
            ("Internal", 465),
            ("RowTooLarge", 467),
            ("Closed", 469),
            ("KeyTooLarge", 471),
        ],
    ),
    (
        "prelude",
        "FatalReason",
        103,
        &[
            ("Malformed", 104),
            ("Quarantined", 105),
            ("MissingFunction", 106),
            ("Storage", 107),
        ],
    ),
    (
        "prelude",
        "Notice",
        108,
        &[
            ("LeaseExpiring", 109),
            ("ShuttingDown", 110),
            ("ConstructorEnabled", 111),
            ("BundleRetired", 112),
            ("BlockingMigration", 113),
            ("UpgradeRequired", 114),
        ],
    ),
    ("prelude", "RejectedRow", 115, &[("RejectedRow", 116)]),
    (
        "prelude",
        "Limit",
        461,
        &[("Time", 462), ("Memory", 463), ("Rounds", 464)],
    ),
    (
        "catalog",
        "TypeDef",
        117,
        &[("SumType", 118), ("Newtype", 119)],
    ),
    ("catalog", "Constructor", 120, &[("Constructor", 121)]),
    (
        "catalog",
        "CtorStage",
        122,
        &[("Declared", 123), ("Enabled", 124), ("Deprecated", 125)],
    ),
    ("catalog", "CtorState", 126, &[("CtorState", 127)]),
    (
        "catalog",
        "Field",
        128,
        &[("Header", 129), ("Payload", 130)],
    ),
    (
        "catalog",
        "RelKind",
        131,
        &[
            ("Base", 132),
            ("Stored", 133),
            ("Catalog", 134),
            ("Volatile", 135),
        ],
    ),
    ("catalog", "Relation", 136, &[("Relation", 137)]),
    ("catalog", "NamedSet", 138, &[("NamedSet", 139)]),
    ("catalog", "SetMember", 140, &[("SetMember", 141)]),
    (
        "catalog",
        "IndexKind",
        142,
        &[
            ("PrimaryKey", 143),
            ("Unique", 144),
            ("Plain", 145),
            ("Interval", 146),
            ("Implied", 466),
        ],
    ),
    ("catalog", "Index", 147, &[("Index", 148)]),
    ("catalog", "IndexCtor", 149, &[("IndexCtor", 150)]),
    ("catalog", "Constraint", 151, &[("Constraint", 152)]),
    (
        "catalog",
        "Function",
        153,
        &[
            ("Language", 154),
            ("Wasm", 155),
            ("Host", 156),
            ("Aggregate", 157),
        ],
    ),
    ("catalog", "WasmModule", 158, &[("WasmModule", 159)]),
    ("catalog", "Extension", 160, &[("Extension", 161)]),
    ("catalog", "Migration", 162, &[("Migration", 163)]),
    (
        "catalog",
        "Priority",
        164,
        &[("AdHoc", 165), ("Background", 166), ("Regular", 167)],
    ),
    (
        "catalog",
        "BundleState",
        168,
        &[("Active", 169), ("Superseded", 170), ("Retired", 171)],
    ),
    ("catalog", "Bundle", 172, &[("Bundle", 173)]),
    (
        "catalog",
        "Registered",
        174,
        &[("Query", 175), ("Txn", 176), ("ImportedFn", 177)],
    ),
    ("catalog", "Feed", 178, &[("Feed", 179)]),
    ("catalog", "FeedMember", 180, &[("FeedMember", 181)]),
    (
        "catalog",
        "PlanState",
        182,
        &[
            ("Current", 183),
            ("Pinned", 184),
            ("Reverted", 185),
            ("Superseded", 186),
        ],
    ),
    ("catalog", "Plan", 187, &[("Plan", 188)]),
    ("catalog", "TzRelease", 189, &[("TzRelease", 190)]),
    ("catalog", "Zone", 191, &[("Zone", 192)]),
    (
        "catalog",
        "Setting",
        193,
        &[
            ("StatementLimit", 194),
            ("TransactionLimit", 195),
            ("IdleLimit", 196),
            ("StreamIdleLimit", 197),
            ("RequestWindow", 198),
            ("SafePointDelay", 199),
            ("ReplicaWait", 200),
            ("DefaultLease", 201),
            ("FeedRetention", 202),
            ("StatsWindow", 203),
            ("StatsRetention", 204),
            ("WaitSampling", 205),
            ("ConflictLog", 206),
            ("VacuumBudget", 207),
            ("ScrubInterval", 208),
            ("LeaseGrace", 346),
            ("BlockingNoticePeriod", 347),
            ("ReplicaVacuumWait", 348),
            ("RetryLimit", 451),
            ("RetentionWindow", 470),
        ],
    ),
    (
        "catalog",
        "AuditEvent",
        209,
        &[
            ("Login", 210),
            ("Grant", 211),
            ("Register", 212),
            ("Retire", 213),
            ("SchemaOp", 214),
            ("Call", 215),
            ("Denied", 216),
            ("Commit", 217),
            ("Truncate", 218),
        ],
    ),
    ("catalog", "ValueState", 349, &[("ValueState", 350)]),
    ("catalog", "Role", 351, &[("Role", 352)]),
    ("catalog", "Credential", 353, &[("Credential", 354)]),
    (
        "catalog",
        "GrantKind",
        355,
        &[("Log", 356), ("Deployment", 357)],
    ),
    ("catalog", "RoleGrant", 358, &[("RoleGrant", 359)]),
    ("catalog", "PeerMapping", 360, &[("PeerMapping", 361)]),
    (
        "catalog",
        "CtorTarget",
        373,
        &[("InRelation", 374), ("OfType", 375)],
    ),
    (
        "catalog",
        "Place",
        376,
        &[("Last", 377), ("Before", 378), ("After", 379)],
    ),
    (
        "catalog",
        "Operator",
        380,
        &[
            ("AddModule", 381),
            ("DropModule", 382),
            ("Install", 383),
            ("Uninstall", 384),
            ("AddType", 385),
            ("DropType", 386),
            ("AddRelation", 387),
            ("DropRelation", 388),
            ("AddSet", 389),
            ("DropSet", 390),
            ("AddConstructor", 391),
            ("RemoveConstructor", 392),
            ("Enable", 393),
            ("Disable", 394),
            ("Deprecate", 395),
            ("RenameConstructor", 396),
            ("RenameType", 397),
            ("RenameRelation", 398),
            ("RenameSet", 399),
            ("RenameField", 400),
            ("Split", 401),
            ("Merge", 402),
            ("Nest", 403),
            ("Unnest", 404),
            ("AddField", 405),
            ("DropField", 406),
            ("AlterField", 468),
            ("MoveField", 407),
            ("AddUnique", 408),
            ("DropUnique", 409),
            ("Rekey", 410),
            ("AddIndex", 411),
            ("DropIndex", 412),
            ("AddDefault", 413),
            ("DropDefault", 414),
            ("DefineFn", 415),
            ("RedefineFn", 416),
            ("DropFn", 417),
            ("DefineAggregate", 418),
            ("DropAggregate", 419),
            ("AddConstraint", 420),
            ("DropConstraint", 421),
            ("AddCheck", 422),
            ("DropCheck", 423),
            ("Store", 424),
            ("Unstore", 425),
            ("AddComputed", 426),
            ("RedefineComputed", 427),
            ("DropComputed", 428),
            ("Grant", 429),
            ("Revoke", 430),
            ("AdoptTzdb", 431),
            ("Script", 432),
            ("Revert", 433),
        ],
    ),
    ("catalog", "MigrationOp", 434, &[("MigrationOp", 435)]),
    (
        "catalog",
        "ArmTarget",
        436,
        &[("OfFn", 437), ("OfConstraint", 438), ("OfComputed", 439)],
    ),
    (
        "catalog",
        "ClauseBody",
        440,
        &[
            ("Joins", 441),
            ("Member", 442),
            ("Arm", 443),
            ("Replacement", 444),
            ("Carry", 445),
            ("Fill", 446),
            ("Privilege", 472),
            ("Scope", 476),
        ],
    ),
    ("catalog", "OpClause", 447, &[("OpClause", 448)]),
    ("catalog", "Default", 452, &[("Default", 453)]),
    ("catalog", "Computed", 454, &[("Computed", 455)]),
    ("catalog", "Term", 458, &[("Term", 459)]),
    ("sys", "Collation", 243, &[("Collation", 244)]),
    ("sys", "CtorStats", 245, &[("CtorStats", 246)]),
    ("sys", "FieldStats", 247, &[("FieldStats", 248)]),
    ("sys", "FieldValue", 249, &[("FieldValue", 250)]),
    ("sys", "Fingerprint", 251, &[("Fingerprint", 252)]),
    ("sys", "QueryStats", 253, &[("QueryStats", 254)]),
    ("sys", "QueryCtorStats", 255, &[("QueryCtorStats", 256)]),
    ("sys", "QueryWindow", 257, &[("QueryWindow", 258)]),
    (
        "sys",
        "SessionState",
        259,
        &[("Idle", 260), ("Busy", 261), ("InTransaction", 262)],
    ),
    ("sys", "Session", 263, &[("Session", 264)]),
    (
        "sys",
        "WaitReason",
        265,
        &[
            ("StorageRead", 266),
            ("FileRead", 267),
            ("LogFlush", 268),
            ("Internal", 269),
            ("SafeSnapshot", 270),
            ("Inserter", 271),
            ("Admission", 272),
        ],
    ),
    (
        "sys",
        "CallState",
        273,
        &[("Queued", 274), ("Running", 275), ("Waiting", 276)],
    ),
    ("sys", "Call", 277, &[("Call", 278)]),
    ("sys", "WaitSample", 279, &[("WaitSample", 280)]),
    ("sys", "WaitTotal", 281, &[("WaitTotal", 282)]),
    ("sys", "Horizon", 283, &[("Horizon", 284)]),
    ("sys", "Conflict", 285, &[("Conflict", 286)]),
    ("sys", "Vacuum", 291, &[("Vacuum", 292)]),
    (
        "sys",
        "JobKind",
        293,
        &[
            ("Reindex", 294),
            ("Compact", 295),
            ("SchemaStage", 296),
            ("Verify", 297),
        ],
    ),
    ("sys", "Job", 298, &[("Job", 299)]),
    ("sys", "PendingDrop", 300, &[("PendingDrop", 301)]),
    ("sys", "RequestMark", 302, &[("RequestMark", 303)]),
    ("sys", "ProcessOption", 304, &[("ProcessOption", 305)]),
    ("sys", "RunId", 310, &[("RunId", 311)]),
    (
        "sys",
        "Finding",
        312,
        &[
            ("Checksum", 313),
            ("StalePage", 314),
            ("Order", 315),
            ("Orphan", 316),
            ("Missing", 317),
            ("Violation", 318),
            ("Structure", 473),
        ],
    ),
    ("sys", "Format", 319, &[("Format", 320)]),
    ("sys", "CommitTime", 366, &[("CommitTime", 367)]),
    ("sys", "SafeSnapshotMark", 368, &[("SafeSnapshotMark", 369)]),
];

/// The fixed ids of the engine-defined relation variables, by module and name.
pub const RELATIONS: &[(&str, &str, u64)] = &[
    ("catalog", "types", 219),
    ("catalog", "constructors", 220),
    ("catalog", "ctor_states", 221),
    ("catalog", "fields", 222),
    ("catalog", "relations", 223),
    ("catalog", "named_sets", 224),
    ("catalog", "set_members", 225),
    ("catalog", "indexes", 226),
    ("catalog", "index_ctors", 227),
    ("catalog", "constraints", 228),
    ("catalog", "functions", 229),
    ("catalog", "wasm_modules", 230),
    ("catalog", "extensions", 231),
    ("catalog", "migrations", 232),
    ("catalog", "bundles", 233),
    ("catalog", "registered", 234),
    ("catalog", "feeds", 235),
    ("catalog", "feed_members", 236),
    ("catalog", "plans", 237),
    ("catalog", "tz_releases", 238),
    ("catalog", "zones", 239),
    ("catalog", "settings", 240),
    ("catalog", "audit", 241),
    ("catalog", "credentials", 242),
    ("catalog", "value_states", 362),
    ("catalog", "roles", 363),
    ("catalog", "grants", 364),
    ("catalog", "peer_mappings", 365),
    ("catalog", "migration_ops", 449),
    ("catalog", "migration_clauses", 450),
    ("catalog", "defaults", 456),
    ("catalog", "computed", 457),
    ("sys", "collations", 321),
    ("sys", "ctor_stats", 322),
    ("sys", "field_stats", 323),
    ("sys", "field_values", 324),
    ("sys", "query_stats", 325),
    ("sys", "query_ctor_stats", 326),
    ("sys", "query_windows", 327),
    ("sys", "sessions", 328),
    ("sys", "calls", 329),
    ("sys", "wait_samples", 330),
    ("sys", "wait_totals", 331),
    ("sys", "horizons", 332),
    ("sys", "conflicts", 333),
    ("sys", "vacuums", 334),
    ("sys", "jobs", 335),
    ("sys", "pending_drops", 336),
    ("sys", "request_marks", 337),
    ("sys", "process_options", 338),
    ("sys", "verify_findings", 339),
    ("sys", "format", 340),
    ("sys", "commit_times", 370),
    ("sys", "safe_snapshots", 371),
];

const _: () = {
    let mut t = 0;
    while t < TYPES.len() {
        let ctors = TYPES[t].3;
        let mut c = 0;
        while c < ctors.len() {
            assert!(
                ctors[c].1 != 0,
                "no constructor holds fixed id 0: a kept row stores no constructor as 0"
            );
            c += 1;
        }
        t += 1;
    }
};

const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Returns the fixed id of constructor `ctor` of type `module::ty` in [`TYPES`]. Fails to compile
/// when [`TYPES`] has no such constructor.
const fn fixed_ctor(module: &str, ty: &str, ctor: &str) -> super::CatCtorId {
    let mut t = 0;
    while t < TYPES.len() {
        let (m, name, _, ctors) = TYPES[t];
        if same(m, module) && same(name, ty) {
            let mut c = 0;
            while c < ctors.len() {
                if same(ctors[c].0, ctor) {
                    return super::CatCtorId::from_raw(ctors[c].1);
                }
                c += 1;
            }
        }
        t += 1;
    }
    panic!("`engine_ctor` names a constructor `TYPES` holds")
}

/// Returns the fixed id of type `module::ty` in [`TYPES`]. Fails to compile when [`TYPES`] has no
/// such type.
const fn fixed_type(module: &str, ty: &str) -> super::CatTypeId {
    let mut t = 0;
    while t < TYPES.len() {
        let (m, name, id, _) = TYPES[t];
        if same(m, module) && same(name, ty) {
            return super::CatTypeId::from_raw(id);
        }
        t += 1;
    }
    panic!("`engine_type` names a type `TYPES` holds")
}

/// Returns the fixed id of relation `module::name` in [`RELATIONS`]. Fails to compile when
/// [`RELATIONS`] has no such relation.
const fn fixed_rel(module: &str, name: &str) -> super::CatRelId {
    let mut r = 0;
    while r < RELATIONS.len() {
        let (m, n, id) = RELATIONS[r];
        if same(m, module) && same(n, name) {
            return super::CatRelId::from_raw(id);
        }
        r += 1;
    }
    panic!("`engine_rel` names a relation `RELATIONS` holds")
}

macro_rules! engine_rels {
    ($($module:literal { $($name:ident $rel:literal),* $(,)? })*) => {
        $($(pub const $name: CatRelId = fixed_rel($module, $rel);)*)*

        /// Every constant of this module with its module's and relation's names.
        #[cfg(test)]
        pub(crate) const NAMED: &[(&str, &str, CatRelId)] = &[$($(($module, $rel, $name),)*)*];
    };
}

/// The fixed ids of the engine-defined relations the engine names.
pub mod engine_rel {
    use super::super::CatRelId;
    use super::fixed_rel;

    engine_rels! {
        "catalog" {
            TYPES "types",
            CONSTRUCTORS "constructors",
            CTOR_STATES "ctor_states",
            VALUE_STATES "value_states",
            FIELDS "fields",
            RELATIONS "relations",
            NAMED_SETS "named_sets",
            SET_MEMBERS "set_members",
            INDEXES "indexes",
            INDEX_CTORS "index_ctors",
            CONSTRAINTS "constraints",
            FUNCTIONS "functions",
            DEFAULTS "defaults",
            COMPUTED "computed",
            PLANS "plans",
            SETTINGS "settings",
            ROLES "roles",
            BUNDLES "bundles",
            REGISTERED "registered",
            FEEDS "feeds",
            FEED_MEMBERS "feed_members",
            GRANTS "grants",
            CREDENTIALS "credentials",
            PEER_MAPPINGS "peer_mappings",
            AUDIT "audit",
            MIGRATIONS "migrations",
            MIGRATION_OPS "migration_ops",
            MIGRATION_CLAUSES "migration_clauses"
        }
        "sys" {
            FIELD_VALUES "field_values",
            SESSIONS "sessions",
            CALLS "calls",
            CONFLICTS "conflicts",
            JOBS "jobs",
            PROCESS_OPTIONS "process_options",
            VERIFY_FINDINGS "verify_findings"
        }
    }
}

macro_rules! engine_types {
    ($($module:literal { $($name:ident $ty:literal),* $(,)? })*) => {
        $($(pub const $name: CatTypeId = fixed_type($module, $ty);)*)*

        /// Every constant of this module with its module's and type's names.
        #[cfg(test)]
        pub(crate) const NAMED: &[(&str, &str, CatTypeId)] = &[$($(($module, $ty, $name),)*)*];
    };
}

/// The fixed ids of the engine-defined types the engine names.
pub mod engine_type {
    use super::super::CatTypeId;
    use super::fixed_type;

    engine_types! {
        "catalog" { SETTING "Setting" }
    }
}

macro_rules! engine_ctors {
    ($($module:literal $ty:literal { $($name:ident $ctor:literal),* $(,)? })*) => {
        $($(pub const $name: CatCtorId = fixed_ctor($module, $ty, $ctor);)*)*

        /// Every constant of this module with its module's, type's and constructor's names.
        #[cfg(test)]
        pub(crate) const NAMED: &[(&str, &str, &str, CatCtorId)] =
            &[$($(($module, $ty, $ctor, $name),)*)*];
    };
}

/// The fixed ids of the engine-defined constructors the engine names.
pub mod engine_ctor {
    use super::super::CatCtorId;
    use super::fixed_ctor;

    engine_ctors! {
        "sys" "JobKind" { JOB_REINDEX "Reindex", JOB_VERIFY "Verify" }
        "catalog" "Role" { ROLE "Role" }
        "catalog" "TypeDef" { TYPE_SUM "SumType", TYPE_NEWTYPE "Newtype" }
        "catalog" "Constructor" { CONSTRUCTOR "Constructor" }
        "catalog" "CtorState" { CTOR_STATE "CtorState" }
        "catalog" "ValueState" { VALUE_STATE "ValueState" }
        "catalog" "Field" { FIELD_HEADER "Header", FIELD_PAYLOAD "Payload" }
        "catalog" "Relation" { RELATION "Relation" }
        "catalog" "NamedSet" { NAMED_SET "NamedSet" }
        "catalog" "SetMember" { SET_MEMBER "SetMember" }
        "catalog" "Index" { INDEX "Index" }
        "catalog" "IndexCtor" { INDEX_CTOR "IndexCtor" }
        "catalog" "Constraint" { CONSTRAINT "Constraint" }
        "catalog" "Function" {
            FUNCTION_LANGUAGE "Language",
            FUNCTION_WASM "Wasm",
            FUNCTION_HOST "Host",
            FUNCTION_AGGREGATE "Aggregate",
        }
        "catalog" "Default" { DEFAULT "Default" }
        "catalog" "Computed" { COMPUTED "Computed" }
        "catalog" "CtorStage" {
            STAGE_DECLARED "Declared",
            STAGE_ENABLED "Enabled",
            STAGE_DEPRECATED "Deprecated",
        }
        "catalog" "RelKind" {
            REL_BASE "Base",
            REL_STORED "Stored",
            REL_CATALOG "Catalog",
            REL_VOLATILE "Volatile",
        }
        "catalog" "IndexKind" {
            INDEX_PRIMARY_KEY "PrimaryKey",
            INDEX_UNIQUE "Unique",
            INDEX_PLAIN "Plain",
            INDEX_INTERVAL "Interval",
            INDEX_IMPLIED "Implied",
        }
        "catalog" "Priority" {
            PRIORITY_AD_HOC "AdHoc",
            PRIORITY_BACKGROUND "Background",
            PRIORITY_REGULAR "Regular",
        }
        "catalog" "BundleState" {
            BUNDLE_ACTIVE "Active",
            BUNDLE_SUPERSEDED "Superseded",
            BUNDLE_RETIRED "Retired",
        }
        "catalog" "Bundle" { BUNDLE "Bundle" }
        "catalog" "Registered" {
            REGISTERED_QUERY "Query",
            REGISTERED_TXN "Txn",
            REGISTERED_IMPORTED_FN "ImportedFn",
        }
        "catalog" "Feed" { FEED "Feed" }
        "catalog" "FeedMember" { FEED_MEMBER "FeedMember" }
        "catalog" "GrantKind" { GRANT_LOG "Log", GRANT_DEPLOYMENT "Deployment" }
        "catalog" "RoleGrant" { ROLE_GRANT "RoleGrant" }
        "catalog" "Credential" { CREDENTIAL "Credential" }
        "catalog" "PeerMapping" { PEER_MAPPING "PeerMapping" }
        "catalog" "Migration" { MIGRATION "Migration" }
        "catalog" "Operator" {
            OP_ADD_MODULE "AddModule",
            OP_DROP_MODULE "DropModule",
            OP_INSTALL "Install",
            OP_UNINSTALL "Uninstall",
            OP_ADD_TYPE "AddType",
            OP_DROP_TYPE "DropType",
            OP_ADD_RELATION "AddRelation",
            OP_DROP_RELATION "DropRelation",
            OP_ADD_SET "AddSet",
            OP_DROP_SET "DropSet",
            OP_ADD_CONSTRUCTOR "AddConstructor",
            OP_REMOVE_CONSTRUCTOR "RemoveConstructor",
            OP_ENABLE "Enable",
            OP_DISABLE "Disable",
            OP_DEPRECATE "Deprecate",
            OP_RENAME_CONSTRUCTOR "RenameConstructor",
            OP_RENAME_TYPE "RenameType",
            OP_RENAME_RELATION "RenameRelation",
            OP_RENAME_SET "RenameSet",
            OP_RENAME_FIELD "RenameField",
            OP_SPLIT "Split",
            OP_MERGE "Merge",
            OP_NEST "Nest",
            OP_UNNEST "Unnest",
            OP_ADD_FIELD "AddField",
            OP_DROP_FIELD "DropField",
            OP_ALTER_FIELD "AlterField",
            OP_MOVE_FIELD "MoveField",
            OP_ADD_UNIQUE "AddUnique",
            OP_DROP_UNIQUE "DropUnique",
            OP_REKEY "Rekey",
            OP_ADD_INDEX "AddIndex",
            OP_DROP_INDEX "DropIndex",
            OP_ADD_DEFAULT "AddDefault",
            OP_DROP_DEFAULT "DropDefault",
            OP_DEFINE_FN "DefineFn",
            OP_REDEFINE_FN "RedefineFn",
            OP_DROP_FN "DropFn",
            OP_DEFINE_AGGREGATE "DefineAggregate",
            OP_DROP_AGGREGATE "DropAggregate",
            OP_ADD_CONSTRAINT "AddConstraint",
            OP_DROP_CONSTRAINT "DropConstraint",
            OP_ADD_CHECK "AddCheck",
            OP_DROP_CHECK "DropCheck",
            OP_STORE "Store",
            OP_UNSTORE "Unstore",
            OP_ADD_COMPUTED "AddComputed",
            OP_REDEFINE_COMPUTED "RedefineComputed",
            OP_DROP_COMPUTED "DropComputed",
            OP_GRANT "Grant",
            OP_REVOKE "Revoke",
            OP_ADOPT_TZDB "AdoptTzdb",
            OP_SCRIPT "Script",
            OP_REVERT "Revert",
        }
        "catalog" "CtorTarget" {
            TARGET_IN_RELATION "InRelation",
            TARGET_OF_TYPE "OfType",
        }
        "catalog" "Place" {
            PLACE_LAST "Last",
            PLACE_BEFORE "Before",
            PLACE_AFTER "After",
        }
        "catalog" "MigrationOp" { MIGRATION_OP "MigrationOp" }
        "catalog" "ArmTarget" {
            ARM_OF_FN "OfFn",
            ARM_OF_CONSTRAINT "OfConstraint",
            ARM_OF_COMPUTED "OfComputed",
        }
        "catalog" "ClauseBody" {
            CLAUSE_JOINS "Joins",
            CLAUSE_MEMBER "Member",
            CLAUSE_ARM "Arm",
            CLAUSE_REPLACEMENT "Replacement",
            CLAUSE_CARRY "Carry",
            CLAUSE_FILL "Fill",
            CLAUSE_PRIVILEGE "Privilege",
            CLAUSE_SCOPE "Scope",
        }
        "catalog" "OpClause" { OP_CLAUSE "OpClause" }
        "prelude" "Bool" { BOOL_FALSE "False", BOOL_TRUE "True" }
        "prelude" "Option" { OPTION_NONE "None", OPTION_SOME "Some" }
        "catalog" "AuditEvent" {
            AUDIT_LOGIN "Login",
            AUDIT_GRANT "Grant",
            AUDIT_REGISTER "Register",
            AUDIT_RETIRE "Retire",
            AUDIT_SCHEMA_OP "SchemaOp",
            AUDIT_CALL "Call",
            AUDIT_DENIED "Denied",
            AUDIT_COMMIT "Commit",
            AUDIT_TRUNCATE "Truncate",
        }
        "catalog" "Setting" {
            SETTING_STATEMENT_LIMIT "StatementLimit",
            SETTING_TRANSACTION_LIMIT "TransactionLimit",
            SETTING_IDLE_LIMIT "IdleLimit",
            SETTING_STREAM_IDLE_LIMIT "StreamIdleLimit",
            SETTING_REQUEST_WINDOW "RequestWindow",
            SETTING_SAFE_POINT_DELAY "SafePointDelay",
            SETTING_REPLICA_WAIT "ReplicaWait",
            SETTING_DEFAULT_LEASE "DefaultLease",
            SETTING_FEED_RETENTION "FeedRetention",
            SETTING_STATS_WINDOW "StatsWindow",
            SETTING_STATS_RETENTION "StatsRetention",
            SETTING_WAIT_SAMPLING "WaitSampling",
            SETTING_CONFLICT_LOG "ConflictLog",
            SETTING_VACUUM_BUDGET "VacuumBudget",
            SETTING_SCRUB_INTERVAL "ScrubInterval",
            SETTING_LEASE_GRACE "LeaseGrace",
            SETTING_BLOCKING_NOTICE_PERIOD "BlockingNoticePeriod",
            SETTING_REPLICA_VACUUM_WAIT "ReplicaVacuumWait",
            SETTING_RETRY_LIMIT "RetryLimit",
            SETTING_RETENTION_WINDOW "RetentionWindow",
        }
        "prelude" "Error" {
            ERROR_RETRY "Retry",
            ERROR_REJECTED "Rejected",
            ERROR_FATAL "Fatal",
        }
        "prelude" "RetryReason" {
            RETRY_SERIALIZATION "Serialization",
            RETRY_SAFE_POINT_TIMEOUT "SafePointTimeout",
            RETRY_OVERLOADED "Overloaded",
            RETRY_STREAM_IDLE "StreamIdle",
            RETRY_FILE_CHANGED "FileChanged",
            RETRY_FENCED "Fenced",
            RETRY_REPLAY_CONFLICT "ReplayConflict",
        }
        "prelude" "RejectReason" {
            REJECT_VIOLATED "Violated",
            REJECT_KEY_VIOLATION "KeyViolation",
            REJECT_KEY_CHANGED "KeyChanged",
            REJECT_NOT_OPEN "NotOpen",
            REJECT_USER_ERROR "UserError",
            REJECT_FAULTED "Faulted",
            REJECT_DENIED "Denied",
            REJECT_INVALID "Invalid",
            REJECT_SCHEMA_REFUSED "SchemaRefused",
            REJECT_UNTRANSLATABLE "Untranslatable",
            REJECT_BAD_INPUT "BadInput",
            REJECT_SCHEMA_MISMATCH "SchemaMismatch",
            REJECT_CANCELLED "Cancelled",
            REJECT_TERMINATED "Terminated",
            REJECT_DEADLINE_EXCEEDED "DeadlineExceeded",
            REJECT_STATEMENT_LIMIT "StatementLimit",
            REJECT_TRANSACTION_EXPIRED "TransactionExpired",
            REJECT_TRANSACTION_BUSY "TransactionBusy",
            REJECT_UNKNOWN_QUERY "UnknownQuery",
            REJECT_UNKNOWN_TRANSACTION "UnknownTransaction",
            REJECT_UNKNOWN_BUNDLE "UnknownBundle",
            REJECT_VERSION_MISMATCH "VersionMismatch",
            REJECT_RETIRED "Retired",
            REJECT_BUNDLE_EXPIRED "BundleExpired",
            REJECT_BUNDLE_STALE "BundleStale",
            REJECT_BUNDLE_SUPERSEDED "BundleSuperseded",
            REJECT_DOWNGRADE_REFUSED "DowngradeRefused",
            REJECT_TOO_OLD "TooOld",
            REJECT_HISTORY_UNAVAILABLE "HistoryUnavailable",
            REJECT_POSITION_NOT_ON_HISTORY "PositionNotOnHistory",
            REJECT_WRONG_VERSION "WrongVersion",
            REJECT_READ_ONLY_REPLICA "ReadOnlyReplica",
            REJECT_REQUEST_EXPIRED "RequestExpired",
            REJECT_REQUEST_REUSED "RequestReused",
            REJECT_AUTH_FAILED "AuthFailed",
            REJECT_UNSUPPORTED_PROTOCOL "UnsupportedProtocol",
            REJECT_UNKNOWN_DATABASE "UnknownDatabase",
            REJECT_CATALOG_MISMATCH "CatalogMismatch",
            REJECT_EDITION_LOCKED "EditionLocked",
            REJECT_UNKNOWN_EDITION "UnknownEdition",
            REJECT_MIXED_VERSIONS "MixedVersions",
            REJECT_MIGRATION_IN_PROGRESS "MigrationInProgress",
            REJECT_DUMP_DAMAGED "DumpDamaged",
            REJECT_BACKUP_DAMAGED "BackupDamaged",
            REJECT_RESTORE_POINT_UNAVAILABLE "RestorePointUnavailable",
            REJECT_INTERNAL "Internal",
            REJECT_ROW_TOO_LARGE "RowTooLarge",
            REJECT_CLOSED "Closed",
            REJECT_KEY_TOO_LARGE "KeyTooLarge",
        }
        "prelude" "FatalReason" {
            FATAL_MALFORMED "Malformed",
            FATAL_QUARANTINED "Quarantined",
            FATAL_MISSING_FUNCTION "MissingFunction",
            FATAL_STORAGE "Storage",
        }
        "prelude" "Limit" {
            LIMIT_TIME "Time",
            LIMIT_MEMORY "Memory",
            LIMIT_ROUNDS "Rounds",
        }
        "prelude" "Position" { POSITION "Position" }
        "prelude" "Origin" {
            ORIGIN_REGISTERED "Registered",
            ORIGIN_FUNCTION "Function",
            ORIGIN_AD_HOC "AdHoc",
        }
        "prelude" "Outcome" { OUTCOME_SUCCEEDED "Succeeded", OUTCOME_FAILED "Failed" }
        "sys" "Finding" {
            FINDING_CHECKSUM "Checksum",
            FINDING_STALE_PAGE "StalePage",
            FINDING_ORDER "Order",
            FINDING_ORPHAN "Orphan",
            FINDING_MISSING "Missing",
            FINDING_VIOLATION "Violation",
            FINDING_STRUCTURE "Structure",
        }
    }
}
