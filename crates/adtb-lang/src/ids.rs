use std::collections::BTreeMap;

use crate::ast::{self, Decl, RelationClause};
use crate::schema::{Schema, TypeDefKind, ValueDef};
use crate::span::{FileId, Span};
use crate::types::*;

/// The catalog ids of a schema's objects, by the handle of each in the schema.
///
/// Every object of a user module has an id, and every object of an engine module its fixed id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogIds {
    pub types: BTreeMap<TypeId, u64>,
    /// Constructors by type and index; a newtype's one constructor is index 0.
    pub ctors: BTreeMap<(TypeId, u32), u64>,
    pub relations: BTreeMap<RelId, u64>,
    /// Fields by type, constructor (`None` for the header) and name.
    pub fields: BTreeMap<(TypeId, Option<u32>, String), u64>,
    pub sets: BTreeMap<(RelId, String), u64>,
    /// The index of each relation's key.
    pub keys: BTreeMap<RelId, u64>,
    /// Uniques and indexes, by relation and position among `RelationDef::indexes`.
    pub indexes: BTreeMap<(RelId, usize), u64>,
    /// The index each reference field implies, by relation, constructor (`None` for the
    /// header) and field.
    pub implied: BTreeMap<(RelId, Option<u32>, String), u64>,
    /// The default a key's `generate` gives.
    pub generated: BTreeMap<RelId, u64>,
    /// Defaults, one for each field a default clause fills, by relation, position among
    /// `RelationDef::defaults`, and the field's constructor (`None` for the header).
    pub defaults: BTreeMap<(RelId, usize, Option<u32>), u64>,
    /// Computed fields, by relation and position among `RelationDef::computed`.
    pub computed: BTreeMap<(RelId, usize), u64>,
    pub functions: BTreeMap<FnId, u64>,
    /// Aggregates, by position among `Schema::aggregates`, in the space of functions.
    pub aggregates: BTreeMap<usize, u64>,
    /// Constraints, by position among `Schema::constraints`.
    pub constraints: BTreeMap<usize, u64>,
    /// Checks, by position among `Schema::checks`, in the space of constraints.
    pub checks: BTreeMap<usize, u64>,
    /// The relation variable of each stored view.
    pub views: BTreeMap<FnId, u64>,
    /// The key index of each stored view.
    pub view_keys: BTreeMap<FnId, u64>,
    /// A stored view's uniques and indexes, by position among its clauses of those kinds.
    pub view_indexes: BTreeMap<(FnId, usize), u64>,
    /// The type of each stored view's rows, for a view whose element type is anonymous.
    pub view_types: BTreeMap<FnId, u64>,
    /// The one constructor of each stored view's row type.
    pub view_ctors: BTreeMap<FnId, u64>,
    /// The fields of a stored view's row constructor, by view and name.
    pub view_fields: BTreeMap<(FnId, String), u64>,
}

/// An id space of catalog objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Space {
    /// Types, constructors and relation variables.
    Shared,
    Field,
    Set,
    Index,
    Constraint,
    Function,
    Default,
    Computed,
}

/// Where an object's id goes in [`CatalogIds`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    Type(TypeId),
    Ctor(TypeId, u32),
    Relation(RelId),
    Field(TypeId, Option<u32>, String),
    Set(RelId, String),
    Key(RelId),
    Index(RelId, usize),
    Implied(RelId, Option<u32>, String),
    Generated(RelId),
    Default(RelId, usize, Option<u32>),
    Computed(RelId, usize),
    Function(FnId),
    Aggregate(usize),
    Constraint(usize),
    Check(usize),
    View(FnId),
    ViewKey(FnId),
    ViewIndex(FnId, usize),
    ViewType(FnId),
    ViewCtor(FnId),
    ViewField(FnId, String),
}

/// One object of a user module, in the order a migration declaring the module files in order creates it.
#[derive(Clone, Debug)]
pub struct Object {
    pub space: Space,
    pub slot: Slot,
    /// The object's name, stable while no operator renames or rewrites it.
    pub path: String,
}

impl CatalogIds {
    pub fn set(&mut self, slot: &Slot, id: u64) {
        match slot.clone() {
            Slot::Type(t) => self.types.insert(t, id),
            Slot::Ctor(t, c) => self.ctors.insert((t, c), id),
            Slot::Relation(r) => self.relations.insert(r, id),
            Slot::Field(t, c, f) => self.fields.insert((t, c, f), id),
            Slot::Set(r, s) => self.sets.insert((r, s), id),
            Slot::Key(r) => self.keys.insert(r, id),
            Slot::Index(r, i) => self.indexes.insert((r, i), id),
            Slot::Implied(r, c, f) => self.implied.insert((r, c, f), id),
            Slot::Generated(r) => self.generated.insert(r, id),
            Slot::Default(r, i, c) => self.defaults.insert((r, i, c), id),
            Slot::Computed(r, i) => self.computed.insert((r, i), id),
            Slot::Function(f) => self.functions.insert(f, id),
            Slot::Aggregate(i) => self.aggregates.insert(i, id),
            Slot::Constraint(i) => self.constraints.insert(i, id),
            Slot::Check(i) => self.checks.insert(i, id),
            Slot::View(f) => self.views.insert(f, id),
            Slot::ViewKey(f) => self.view_keys.insert(f, id),
            Slot::ViewIndex(f, i) => self.view_indexes.insert((f, i), id),
            Slot::ViewType(f) => self.view_types.insert(f, id),
            Slot::ViewCtor(f) => self.view_ctors.insert(f, id),
            Slot::ViewField(f, n) => self.view_fields.insert((f, n), id),
        };
    }
}

/// The first id the log gives in every space.
pub const FIRST_LOG_ID: u64 = 1024;

/// Returns the ids of a schema built from `files`: the engine's fixed ones, and for the user modules
/// those one migration declaring every declaration of `files` in order, as `add` operators, gives.
pub fn declared(schema: &Schema, files: &[(FileId, ast::SchemaFile)]) -> CatalogIds {
    let mut ids = engine(schema);
    let mut next: BTreeMap<Space, u64> = BTreeMap::new();
    for o in objects(schema, files, &|_, _| String::new()) {
        let n = next.entry(o.space).or_insert(FIRST_LOG_ID);
        ids.set(&o.slot, *n);
        *n += 1;
    }
    ids
}

fn engine(schema: &Schema) -> CatalogIds {
    let mut ids = CatalogIds::default();
    for (i, t) in schema.types.iter().enumerate() {
        let m = &schema.module(t.module).name;
        if !schema.module(t.module).engine {
            continue;
        }
        let Some((_, _, id, ctors)) = TYPES.iter().find(|(tm, tn, _, _)| tm == m && *tn == t.name)
        else {
            continue;
        };
        let def = TypeId(i as u32);
        ids.types.insert(def, *id);
        let newtype = matches!(t.kind, TypeDefKind::Newtype { .. });
        for (c, cid) in *ctors {
            match t.ctor_index(c) {
                Some(k) => {
                    ids.ctors.insert((def, k), *cid);
                }
                None if newtype => {
                    ids.ctors.insert((def, 0), *cid);
                }
                None => {}
            }
        }
    }
    for (i, r) in schema.relations.iter().enumerate() {
        let m = &schema.module(r.module).name;
        if let Some((_, _, id)) = RELATIONS
            .iter()
            .find(|(rm, rn, _)| rm == m && *rn == r.name)
        {
            ids.relations.insert(RelId(i as u32), *id);
        }
    }
    let mut types: Vec<(u64, TypeId)> = ids.types.iter().map(|(&t, &id)| (id, t)).collect();
    types.sort();
    let mut field = 0;
    let mut added = Vec::new();
    for (_, def) in types {
        let td = schema.ty(def);
        let parts = td.header().iter().map(|f| (None, f)).chain(
            td.ctors()
                .iter()
                .enumerate()
                .flat_map(|(k, c)| c.fields.iter().map(move |f| (Some(k as u32), f))),
        );
        for (c, f) in parts {
            // Constructors from 471 extend the original engine field-id allocation.
            if c.is_some_and(|c| ids.ctors[&(def, c)] >= 471) {
                added.push((def, c, f.name.clone()));
                continue;
            }
            ids.fields.insert((def, c, f.name.clone()), field);
            field += 1;
        }
    }
    for slot in added {
        ids.fields.insert(slot, field);
        field += 1;
    }
    let mut relations: Vec<(u64, RelId)> = ids.relations.iter().map(|(&r, &id)| (id, r)).collect();
    relations.sort();
    let mut index = 0;
    for (_, r) in relations {
        ids.keys.insert(r, index);
        index += 1;
        for i in 0..schema.relation(r).indexes.len() {
            ids.indexes.insert((r, i), index);
            index += 1;
        }
    }
    ids
}

/// Every object the user modules of `files` declare, in creation order, `text` giving the source
/// text of a span of a file for the paths of uniques and indexes.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per declaration kind; each arm would have to be a function"
)]
pub fn objects(
    schema: &Schema,
    files: &[(FileId, ast::SchemaFile)],
    text: &dyn Fn(FileId, Span) -> String,
) -> Vec<Object> {
    let mut out = Vec::new();
    let mut push = |space: Space, slot: Slot, path: String| out.push(Object { space, slot, path });
    for (file, sf) in files {
        if file.is_engine() {
            continue;
        }
        let Some(mid) = schema.module_named(&sf.module.text) else {
            continue;
        };
        let m = &sf.module.text;
        let values = &schema.module(mid).values;
        for d in &sf.decls {
            match d {
                Decl::Type(t) => {
                    let def = if t.name.text == "Principal" {
                        Some(schema.principal)
                    } else {
                        schema.module(mid).types.get(&t.name.text).copied()
                    };
                    let Some(def) = def else { continue };
                    let td = schema.ty(def);
                    let tn = &td.name;
                    push(Space::Shared, Slot::Type(def), format!("type {m}::{tn}"));
                    if matches!(td.kind, TypeDefKind::Newtype { .. }) {
                        push(
                            Space::Shared,
                            Slot::Ctor(def, 0),
                            format!("ctor {m}::{tn}.{tn}"),
                        );
                    }
                    for (k, c) in td.ctors().iter().enumerate() {
                        push(
                            Space::Shared,
                            Slot::Ctor(def, k as u32),
                            format!("ctor {m}::{tn}.{}", c.name),
                        );
                    }
                    for f in td.header() {
                        push(
                            Space::Field,
                            Slot::Field(def, None, f.name.clone()),
                            format!("field {m}::{tn}.{}", f.name),
                        );
                    }
                    for (k, c) in td.ctors().iter().enumerate() {
                        for f in &c.fields {
                            push(
                                Space::Field,
                                Slot::Field(def, Some(k as u32), f.name.clone()),
                                format!("field {m}::{tn}.{}.{}", c.name, f.name),
                            );
                        }
                    }
                }
                Decl::Relation(r) => {
                    let Some(&ValueDef::Relation(rel)) = values.get(&r.name.name.text) else {
                        continue;
                    };
                    let rn = &r.name.name.text;
                    push(
                        Space::Shared,
                        Slot::Relation(rel),
                        format!("relation {m}::{rn}"),
                    );
                    let name = format!("{m}::{rn}");
                    clause_objects(
                        &mut push,
                        schema,
                        *file,
                        text,
                        &r.clauses,
                        &name,
                        Owner::Relation(rel),
                    );
                    let def = schema.ty(schema.relation(rel).elem_def);
                    for i in schema.implied_indexes(rel) {
                        let at = i.ctor.map_or(String::new(), |c| {
                            format!("{}.", def.ctors()[c as usize].name)
                        });
                        push(
                            Space::Index,
                            Slot::Implied(rel, i.ctor, i.field.clone()),
                            format!("index {name} implied by {at}{}", i.field),
                        );
                    }
                }
                Decl::Set(s) => {
                    if let Some(&ValueDef::Relation(rel)) = values.get(&s.relation.text) {
                        push(
                            Space::Set,
                            Slot::Set(rel, s.name.text.clone()),
                            format!("set {m}::{}.{}", s.relation.text, s.name.text),
                        );
                    }
                }
                Decl::Fn(f) => {
                    if let Some(&ValueDef::Function(id)) = values.get(&f.name.text) {
                        push(
                            Space::Function,
                            Slot::Function(id),
                            format!("fn {m}::{}", f.name.text),
                        );
                    }
                }
                Decl::Aggregate(a) => {
                    if let Some(&ValueDef::Aggregate(i)) = values.get(&a.name.text) {
                        push(
                            Space::Function,
                            Slot::Aggregate(i as usize),
                            format!("fn {m}::{}", a.name.text),
                        );
                    }
                }
                Decl::Constraint(c) => {
                    if let Some(&ValueDef::Constraint(i)) = values.get(&c.name.text) {
                        push(
                            Space::Constraint,
                            Slot::Constraint(i as usize),
                            format!("constraint {m}::{}", c.name.text),
                        );
                    }
                }
                Decl::Check(c) => {
                    if let Some(&ValueDef::Check(i)) = values.get(&c.name.text) {
                        push(
                            Space::Constraint,
                            Slot::Check(i as usize),
                            format!("constraint {m}::{}", c.name.text),
                        );
                    }
                }
                Decl::Store(s) => {
                    let Some(&ValueDef::Function(f)) = values.get(&s.view.name.text) else {
                        continue;
                    };
                    let vn = &s.view.name.text;
                    push(Space::Shared, Slot::View(f), format!("relation {m}::{vn}"));
                    if let Some(fields) = anonymous_row(files, m, vn) {
                        push(Space::Shared, Slot::ViewType(f), format!("type {m}::{vn}"));
                        push(
                            Space::Shared,
                            Slot::ViewCtor(f),
                            format!("ctor {m}::{vn}.{vn}"),
                        );
                        for name in fields {
                            push(
                                Space::Field,
                                Slot::ViewField(f, name.clone()),
                                format!("field {m}::{vn}.{vn}.{name}"),
                            );
                        }
                    }
                    let name = format!("{m}::{vn}");
                    clause_objects(
                        &mut push,
                        schema,
                        *file,
                        text,
                        &s.clauses,
                        &name,
                        Owner::View(f),
                    );
                }
            }
        }
    }
    out
}

/// The fields of the rows of view `view` of module `module`, as its result type writes them, when
/// its element type is anonymous: none for an anonymous sum or `()`; `None` for a declared type.
fn anonymous_row(
    files: &[(FileId, ast::SchemaFile)],
    module: &str,
    view: &str,
) -> Option<Vec<String>> {
    let result = files
        .iter()
        .filter(|(_, sf)| sf.module.text == module)
        .flat_map(|(_, sf)| &sf.decls)
        .find_map(|d| match d {
            Decl::Fn(f) if f.name.text == view => Some(&f.result),
            _ => None,
        });
    let Some(ast::Type {
        kind: ast::TypeKind::Named { args, .. },
        ..
    }) = result
    else {
        return None;
    };
    let [ast::TypeArg::Type(elem)] = args.as_slice() else {
        return None;
    };
    match &elem.kind {
        ast::TypeKind::Record(fs) => Some(fs.iter().map(|f| f.name.text.clone()).collect()),
        ast::TypeKind::Sum(_) | ast::TypeKind::Unit => Some(Vec::new()),
        _ => None,
    }
}

/// Returns the fields of relation `r` a default of `field`, of constructor `ctor` when one is
/// named, fills, each by its constructor: `None` for the header's field, else each constructor
/// with the field in declaration order.
fn default_fields(
    schema: &Schema,
    r: RelId,
    ctor: Option<&ast::Ident>,
    field: &str,
) -> Vec<Option<u32>> {
    let def = schema.relation(r).elem_def;
    if def.0 == u32::MAX {
        return Vec::new();
    }
    let td = schema.ty(def);
    if td.header().iter().any(|f| f.name == field) {
        return vec![None];
    }
    td.ctors()
        .iter()
        .enumerate()
        .filter(|(_, c)| ctor.is_none_or(|n| n.text == c.name))
        .filter(|(_, c)| c.fields.iter().any(|f| f.name == field))
        .map(|(k, _)| Some(k as u32))
        .collect()
}

/// What declares a run of clauses.
#[derive(Clone, Copy)]
enum Owner {
    Relation(RelId),
    View(FnId),
}

/// The objects a relation's or a stored view's clauses create, in written order.
fn clause_objects(
    push: &mut dyn FnMut(Space, Slot, String),
    schema: &Schema,
    file: FileId,
    text: &dyn Fn(FileId, Span) -> String,
    clauses: &[RelationClause],
    name: &str,
    owner: Owner,
) {
    let (mut ix, mut dx, mut cx) = (0, 0, 0);
    for c in clauses {
        match (c, owner) {
            (RelationClause::Key(_, generate), _) => {
                let key = match owner {
                    Owner::Relation(r) => Slot::Key(r),
                    Owner::View(f) => Slot::ViewKey(f),
                };
                push(Space::Index, key, format!("index {name} key"));
                if let (Some(_), Owner::Relation(r)) = (generate, owner) {
                    push(
                        Space::Default,
                        Slot::Generated(r),
                        format!("default {name} key"),
                    );
                }
            }
            (
                RelationClause::Unique(fs, scope)
                | RelationClause::Index {
                    fields: fs, scope, ..
                },
                _,
            ) => {
                let kind = match c {
                    RelationClause::Unique(..) => "unique",
                    RelationClause::Index { interval: true, .. } => "interval index",
                    _ => "index",
                };
                let fields: Vec<String> = fs.iter().map(|e| text(file, e.span)).collect();
                let scope = match scope {
                    None => "*".to_string(),
                    Some(ast::CtorScope::One(c)) => c.text.clone(),
                    Some(ast::CtorScope::Many(cs)) => cs
                        .iter()
                        .map(|c| c.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" | "),
                };
                let slot = match owner {
                    Owner::Relation(r) => Slot::Index(r, ix),
                    Owner::View(f) => Slot::ViewIndex(f, ix),
                };
                push(
                    Space::Index,
                    slot,
                    format!("index {name} {kind} ({}) in {scope}", fields.join(", ")),
                );
                ix += 1;
            }
            (RelationClause::Default { ctor, field, .. }, Owner::Relation(r)) => {
                for c in default_fields(schema, r, ctor.as_ref(), &field.text) {
                    let at = c.map_or(String::new(), |k| {
                        let td = schema.ty(schema.relation(r).elem_def);
                        format!("{}.", td.ctors()[k as usize].name)
                    });
                    push(
                        Space::Default,
                        Slot::Default(r, dx, c),
                        format!("default {name} {at}{}", field.text),
                    );
                }
                dx += 1;
            }
            (RelationClause::Computed(n, _, _), Owner::Relation(r)) => {
                push(
                    Space::Computed,
                    Slot::Computed(r, cx),
                    format!("computed {name}.{}", n.text),
                );
                cx += 1;
            }
            _ => {}
        }
    }
}

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
        "Principal",
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
