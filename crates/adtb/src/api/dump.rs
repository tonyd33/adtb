//! The command `dump`, and the restore of a dump into a new database file.

use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::kernel::{Kind, Terms};
use adtb_lang::schema::{Generator, Schema};
use adtb_lang::textual::{Form, Header, print, read_document};
use adtb_lang::types::{RelId, Ty};

use crate::catalog::{self, Dormant, OWNER_ROLE, layout, values};
use crate::exec::Val;
use crate::runtime::Runtime;
use crate::storage::{self, BackupError, DumpManifest, Envelope};
use crate::write::{Bounds, Local, Text, Txn};

use super::backup::{CATALOG_VERSION, RestorePoint, refused};
use super::bundle::EDITION;
use super::error::{FatalReason, RejectReason};
use super::{
    CallOptions, CreateOptions, Database, Engine, Error, Log, OWNER_ID, OpenError, OpenOptions,
    Position,
};

const LOG: &str = "log";
const IDS: &str = "ids";
const GENERATORS: &str = "generators";
const SECURITY: &str = "security";
const SETTINGS: &str = "settings";
const AUDIT: &str = "audit";

/// The catalog relations holding the key of every catalog object but a constructor.
const KEYED: [&str; 9] = [
    "types",
    "fields",
    "relations",
    "named_sets",
    "indexes",
    "constraints",
    "functions",
    "defaults",
    "computed",
];
/// The catalog relations of the `security` entry, whose `grants` are its `Deployment` rows.
const SECURED: [&str; 4] = ["roles", "credentials", "grants", "peer_mappings"];
/// The catalog relations whose keys the engine draws from a counter of its own.
const DRAWN: [&str; 3] = ["roles", "grants", "audit"];

fn damaged(part: &str) -> Error {
    Error::rejected(RejectReason::DumpDamaged {
        part: part.to_owned(),
    })
}

fn dump_refused(e: BackupError) -> Error {
    match e {
        BackupError::Damaged { part } => damaged(&part),
        other => refused(other),
    }
}

fn header(e: &Envelope) -> Header {
    Header {
        edition: e.edition.clone(),
        version: e.schema_version,
        hash: e.schema_hash.clone(),
    }
}

fn header_line(e: &Envelope) -> String {
    format!(
        "text {} pins {} ({})\n",
        e.edition, e.schema_version, e.schema_hash
    )
}

/// Returns the relations a dump holds the rows of: every stored relation of a user module.
fn dumped(schema: &Schema) -> impl Iterator<Item = RelId> + '_ {
    (0..schema.relations.len())
        .map(|i| RelId(u32::try_from(i).expect("a schema has fewer than 2^32 relations")))
        .filter(|&r| {
            let rel = schema.relation(r);
            !rel.volatile && !schema.module(rel.module).engine
        })
}

fn catalog_relations(schema: &Schema, names: &[&str]) -> Vec<RelId> {
    names.iter().map(|n| values::relation(schema, n)).collect()
}

/// Returns the relations whose next key a `generators` entry holds, in ascending order.
fn generated(schema: &Schema) -> Vec<RelId> {
    let mut out: Vec<RelId> = dumped(schema).filter(|&r| sequenced(schema, r)).collect();
    out.extend(catalog_relations(schema, &DRAWN));
    out.sort();
    out
}

/// Returns field `name` of `row`, a row of `r`.
fn field<'v>(schema: &Schema, r: RelId, row: &'v Val, name: &str) -> Option<&'v Val> {
    let Val::Ctor(c, fields) = row else {
        return None;
    };
    let at = layout::row(schema, r, *c)
        .iter()
        .position(|(n, _)| n == name)?;
    fields.get(at)
}

/// Returns whether `row` of `catalog::grants` is a `Deployment` row.
fn deployment(schema: &Schema, grants: RelId, row: &Val) -> bool {
    let fields = layout::row(schema, grants, 0);
    let Some((_, Ty::Data { def, .. })) = fields.iter().find(|(n, _)| n == "kind") else {
        panic!("catalog RoleGrant declares its GrantKind as `kind`");
    };
    let kind = schema.ty(*def).ctor_index("Deployment");
    matches!(field(schema, grants, row, "kind"), Some(Val::Ctor(c, _)) if Some(*c) == kind)
}

/// Returns the rows of `r` as a document: `head`, a `data` line naming `r` with its module, then
/// one row per line in ascending order.
fn document(schema: &Schema, r: RelId, rows: &BTreeSet<Val>, head: &str) -> String {
    let rel = schema.relation(r);
    let form = Form::of(schema, &rel.elem);
    let module = &schema.module(rel.module).name;
    let mut out = format!("{head}data {module}::{}\n", rel.name);
    for row in rows {
        let line = print(row, &form)
            .unwrap_or_else(|e| panic!("every stored value has a textual form: {e:?}"));
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Returns an entry holding `rows`, a document for each relation.
fn sections_text(e: &Envelope, schema: &Schema, rows: &Rows) -> String {
    let mut out = header_line(e);
    for (&r, rows) in rows {
        out.push_str(&document(schema, r, rows, ""));
    }
    out
}

/// The rows of each of several relations.
type Rows = BTreeMap<RelId, BTreeSet<Val>>;

fn rows_of(
    schema: &Schema,
    names: &[&str],
    mut rows: impl FnMut(RelId) -> Result<Vec<Val>, Error>,
) -> Result<Rows, Error> {
    catalog_relations(schema, names)
        .into_iter()
        .map(|r| Ok((r, rows(r)?.into_iter().collect())))
        .collect()
}

/// Returns the rows an entry of [`sections_text`] holds, once it holds a document for exactly
/// the relations `names`.
fn read_sections(schema: &Schema, text: &str, e: &Envelope, names: &[&str]) -> Option<Rows> {
    let rest = text.strip_prefix(&header_line(e))?;
    let mut sections: Vec<String> = Vec::new();
    for line in rest.split_inclusive('\n') {
        if line.starts_with("data ") {
            sections.push(String::new());
        }
        sections.last_mut()?.push_str(line);
    }
    let mut out = Rows::new();
    for section in &sections {
        let doc = read_document::<Val>(schema, section).ok()?;
        out.insert(doc.relation, doc.rows).is_none().then_some(())?;
    }
    let wanted: BTreeSet<RelId> = catalog_relations(schema, names).into_iter().collect();
    out.keys().copied().eq(wanted).then_some(out)
}

fn log_text(e: &Envelope, log: &[adtb_lang::evolve::Logged]) -> String {
    let mut out = header_line(e);
    for l in log {
        out.push_str(&format!(
            "migration {} {}\n{}\n",
            l.number,
            l.text.len(),
            l.text
        ));
    }
    out
}

/// Returns the texts of the migrations of a `log` entry, in version order from version 1.
fn read_log(text: &str, e: &Envelope) -> Option<Vec<String>> {
    let mut rest = text.strip_prefix(&header_line(e))?;
    let mut out = Vec::new();
    while !rest.is_empty() {
        let (line, after) = rest.split_once('\n')?;
        let (version, len) = line.strip_prefix("migration ")?.split_once(' ')?;
        let (version, len): (u64, usize) = (version.parse().ok()?, len.parse().ok()?);
        if version != out.len() as u64 + 1 {
            return None;
        }
        let body = after.get(..len)?;
        rest = after.get(len..)?.strip_prefix('\n')?;
        out.push(body.to_owned());
    }
    (out.len() as u64 == e.schema_version).then_some(out)
}

/// Returns the next value of each key generator of a `generators` entry, by catalog relation id.
fn read_generators(text: &str, e: &Envelope) -> Option<Vec<(u64, i64)>> {
    let rest = text.strip_prefix(&header_line(e))?;
    rest.lines()
        .map(|l| {
            let (rel, next) = l.strip_prefix("generator ")?.split_once(' ')?;
            Some((rel.parse().ok()?, next.parse().ok()?))
        })
        .collect()
}

fn sequenced(schema: &Schema, r: RelId) -> bool {
    let rel = schema.relation(r);
    rel.generate == Some(Generator::Sequence) && rel.key.len() == 1
}

fn data_name(schema: &Schema, r: RelId, ctor: u32) -> String {
    let rel = schema.relation(r);
    let ctor = &schema.ty(rel.elem_def).ctors()[ctor as usize].name;
    let module = &schema.module(rel.module).name;
    format!("data/{module}::{}.{ctor}.txt", rel.name)
}

fn ctor_of(v: &Val) -> u32 {
    match v {
        Val::Ctor(c, _) => *c,
        _ => 0,
    }
}

impl<R: Runtime> Engine<R> {
    /// Write the database as `tx` reads it to the dump directory `path`. Returns its manifest.
    pub(super) fn dump_to(&self, tx: &Txn, path: &str) -> Result<DumpManifest, Error> {
        let target = self.target(path)?;
        let envelope = self.envelope();
        let failed = |e| self.write_error(super::TxnId(0), e);
        let rows = |r| self.db.relation_rows(tx, r).map_err(failed);
        let schema = &*self.schema;
        let mut security = rows_of(schema, &SECURED, rows)?;
        let grants = values::relation(schema, "grants");
        security
            .entry(grants)
            .or_default()
            .retain(|g| deployment(schema, grants, g));
        let mut ids = rows_of(schema, &KEYED, rows)?;
        let mut logged: BTreeSet<Val> = rows(grants)?.into_iter().collect();
        logged.retain(|g| !deployment(schema, grants, g));
        ids.insert(grants, logged);
        let mut entries = vec![(LOG.to_owned(), log_text(&envelope, &self.source.log))];
        for (name, rows) in [
            (IDS, ids),
            (SECURITY, security),
            (SETTINGS, rows_of(schema, &[SETTINGS], rows)?),
            (AUDIT, rows_of(schema, &[AUDIT], rows)?),
        ] {
            entries.push((name.to_owned(), sections_text(&envelope, schema, &rows)));
        }
        let mut generators = header_line(&envelope);
        for r in generated(schema) {
            let next = self.db.sequence_next(r);
            let id = schema.ids.relations[&r];
            generators.push_str(&format!("generator {id} {next}\n"));
        }
        entries.push((GENERATORS.to_owned(), generators));
        for r in dumped(schema) {
            let mut by_ctor: BTreeMap<u32, BTreeSet<Val>> = BTreeMap::new();
            for row in self.db.relation_rows(tx, r).map_err(failed)? {
                by_ctor.entry(ctor_of(&row)).or_default().insert(row);
            }
            let ctors = schema.ty(schema.relation(r).elem_def).ctors().len();
            for c in 0..u32::try_from(ctors).expect("a sum has fewer than 2^32 constructors") {
                let rows = by_ctor.remove(&c).unwrap_or_default();
                let text = document(schema, r, &rows, &header_line(&envelope));
                entries.push((data_name(schema, r, c), text));
            }
        }
        let entries: Vec<(String, Vec<u8>)> = entries
            .into_iter()
            .map(|(name, text)| (name, text.into_bytes()))
            .collect();
        let snapshot = self.db.read_position(tx);
        storage::write_dump(&self.rt, &target, envelope, snapshot, &entries).map_err(refused)
    }
}

/// Restore the dump at `source` as the new database file `path` of runtime `rt`, which opens with
/// `options`. Returns the dump's manifest and each dormant grant of the restored file.
pub(super) fn restore_dump<R: Runtime>(
    rt: &R,
    source: &str,
    path: &str,
    to: RestorePoint,
    options: &OpenOptions,
) -> Result<(DumpManifest, Vec<Dormant>), Error> {
    let (manifest, entries) = storage::read_dump(rt, source).map_err(dump_refused)?;
    let e = &manifest.envelope;
    if e.edition != EDITION || e.catalog_version > CATALOG_VERSION {
        return Err(Error::invalid(format!(
            "this engine does not read a dump of edition {} and catalog version {}",
            e.edition, e.catalog_version
        )));
    }
    let snapshot = Position::from(manifest.snapshot);
    if to != RestorePoint::Latest && to != RestorePoint::Position(snapshot) {
        return Err(Error::rejected(RejectReason::RestorePointUnavailable {
            oldest: snapshot,
            newest: snapshot,
        }));
    }
    let entry = |name: &str| {
        entries
            .get(name)
            .and_then(|b| std::str::from_utf8(b).ok())
            .ok_or_else(|| damaged(name))
    };
    let log = read_log(entry(LOG)?, e).ok_or_else(|| damaged(LOG))?;
    let generators = read_generators(entry(GENERATORS)?, e).ok_or_else(|| damaged(GENERATORS))?;
    let schema = catalog::engine_schema();
    let section = |name: &str, names: &[&str]| {
        read_sections(&schema, entry(name)?, e, names).ok_or_else(|| damaged(name))
    };
    let security = entries
        .get(SECURITY)
        .map(|_| security_rows(&schema, entry(SECURITY)?, e));
    let security = security.transpose()?.unwrap_or_default();
    let deployed = section(SETTINGS, &[SETTINGS])?.into_iter();
    let deployed = deployed.chain(section(AUDIT, &[AUDIT])?).collect();
    let mut ids = section(IDS, &[&KEYED[..], &["grants"]].concat())?;
    let grants = values::relation(&schema, "grants");
    let logged = ids.remove(&grants).unwrap_or_default();
    let key = |g| dormant_grant(&schema, grants, g).map(|d| (d.grantee, d.privilege, d.target));
    let held = logged
        .iter()
        .filter(|g| dormant(&schema, grants, g))
        .map(key);
    let held = held.collect::<Option<_>>().ok_or_else(|| damaged(IDS))?;
    let parsed = Parsed {
        schema,
        ids,
        logged,
        deployed,
    };
    let log_dir = options.storage(path).log_dir();
    let (aside, made) = storage::restore_aside(rt, path, &log_dir).map_err(refused)?;
    let aside_log = options.storage(&aside).log_dir();
    let creating = storage::creating(&aside);
    let opening = OpenOptions {
        create: Some(CreateOptions {
            after: Some(manifest.snapshot),
            replay: Some(held),
        }),
        ..options.clone()
    };
    let open = |log: Log, options: &OpenOptions| {
        Database::open_in(rt.clone(), &aside, &log, options).map_err(|e| match e {
            OpenError::HostFnMissing { name } => Error::Fatal {
                reason: FatalReason::MissingFunction { function: name },
            },
            OpenError::Io(e) => Error::storage(format!("`{aside}`: {e}")),
            OpenError::MigrationRefused { .. } => damaged(LOG),
            e => Error::invalid(format!("the dumped log does not open: {e}")),
        })
    };
    let bare = OpenOptions {
        registrations: Vec::new(),
        ..opening.clone()
    };
    let built = open(Log::default(), &bare)
        .and_then(|db| {
            let secured = db
                .engine
                .replace_rows(on(&parsed.schema, &db.engine.schema, security));
            let closed = db.close();
            secured.and(closed)
        })
        .and_then(|()| open(Log::new(log), &opening))
        .and_then(|db| {
            let filled = fill(&db, &manifest, &entries, parsed, &generators);
            let closed = db.close();
            filled.and_then(|dormant| closed.map(|()| dormant))
        })
        .and_then(|dormant| {
            storage::publish_database(rt, (&aside, &aside_log), path, &log_dir)
                .map(|()| dormant)
                .map_err(refused)
        });
    if built.is_err() {
        storage::discard_aside(rt, &[&aside, &aside_log, &creating], &made);
    }
    built.map(|dormant| (manifest, dormant))
}

/// Returns the rows of `text`, a `security` entry, by relation of `schema`, the schema of a file
/// holding no migration.
///
/// Fail with `DumpDamaged` naming `security` when its schema owner has another name, or its
/// grants hold a row of the log.
fn security_rows(schema: &Schema, text: &str, e: &Envelope) -> Result<Rows, Error> {
    let rows = read_sections(schema, text, e, &SECURED).ok_or_else(|| damaged(SECURITY))?;
    let [roles, grants] = [
        values::relation(schema, "roles"),
        values::relation(schema, "grants"),
    ];
    let owner = rows[&roles]
        .iter()
        .find(|r| matches!(field(schema, roles, r, "id"), Some(Val::Int(OWNER_ID))));
    let named = owner.and_then(|r| field(schema, roles, r, "name"));
    if !matches!(named, Some(Val::Text(n)) if &**n == OWNER_ROLE)
        || !rows[&grants].iter().all(|g| deployment(schema, grants, g))
    {
        return Err(damaged(SECURITY));
    }
    Ok(rows)
}

impl<R: Runtime> Engine<R> {
    /// Replace every row of each relation of `rows` by its rows, as one transaction.
    ///
    /// Precondition: each relation is one no migration writes.
    fn replace_rows(&self, rows: Rows) -> Result<(), Error> {
        let mut terms = Terms::default();
        let mut params = Vec::new();
        let mut changes = Vec::new();
        for (r, rows) in rows {
            let set = Ty::Set(Box::new(Ty::Data {
                def: self.schema.relation(r).elem_def,
                args: Vec::new(),
                ctors: None,
            }));
            let x = terms.var("row·", set.clone());
            params.push((x, Val::set(rows)));
            let gone = terms.add(Kind::Rel(r, None), set.clone());
            changes.push(terms.add(Kind::Delete(r, gone), Ty::Change));
            let new = terms.add(Kind::Var(x), set);
            changes.push(terms.add(Kind::Insert(r, new), Ty::Change));
        }
        let all = terms.add(Kind::Par(changes), Ty::Change);
        let text = Text {
            schema: &self.schema,
            terms: &terms,
            items: &[all],
            params: &params,
            clock: self.clock(),
            caller: None,
        };
        let failed = |w| self.write_error(super::TxnId(0), w);
        let tx = self.db.begin().map_err(failed)?;
        match self
            .db
            .statements(&tx, text, &mut Local::default(), Bounds::default())
        {
            Ok(_) => self.db.commit(tx).map(drop).map_err(failed),
            Err(f) => {
                self.db.abort(tx).map_err(failed)?;
                Err(failed(f.error))
            }
        }
    }
}

/// Check the opened database against the dump's version, parts and `ids`, insert every dumped row
/// as one change set, and resume every generator. Returns each dormant grant it wrote.
fn fill<R: Runtime>(
    db: &Database<R>,
    manifest: &DumpManifest,
    entries: &BTreeMap<String, Vec<u8>>,
    parsed: Parsed,
    generators: &[(u64, i64)],
) -> Result<Vec<Dormant>, Error> {
    let engine = &*db.engine;
    let e = &manifest.envelope;
    if header(&engine.envelope()) != header(e) {
        return Err(damaged(LOG));
    }
    let schema = &*engine.schema;
    let data: Vec<(RelId, u32, String)> = dumped(schema)
        .flat_map(|r| {
            let ctors = schema.ty(schema.relation(r).elem_def).ctors().len();
            (0..u32::try_from(ctors).expect("a sum has fewer than 2^32 constructors"))
                .map(move |c| (r, c, data_name(schema, r, c)))
        })
        .collect();
    if let Some((_, _, missing)) = data.iter().find(|(_, _, n)| !entries.contains_key(n)) {
        return Err(damaged(missing));
    }
    let named = [LOG, IDS, GENERATORS, SECURITY, SETTINGS, AUDIT];
    let expected = |n: &str| named.contains(&n) || data.iter().any(|d| d.2 == n);
    if let Some(unexpected) = entries.keys().find(|n| !expected(n)) {
        return Err(damaged(unexpected));
    }
    let read = |r: RelId| {
        let rel = schema.relation(r);
        let text = format!("{}::{}", schema.module(rel.module).name, rel.name);
        let rows = db.query_text(&text, &[], &CallOptions::default())?.pop();
        rows.into_iter()
            .flatten()
            .collect::<Result<Vec<Val>, Error>>()
    };
    let grants = values::relation(schema, "grants");
    let kept = paired(schema, grants, read(grants)?, parsed.logged).ok_or_else(|| damaged(IDS))?;
    if on(&parsed.schema, schema, parsed.ids) != rows_of(schema, &KEYED, read)? {
        return Err(damaged(IDS));
    }
    let mut deployed = on(&parsed.schema, schema, parsed.deployed);
    deployed.insert(grants, kept);
    let resumed: BTreeMap<RelId, i64> = generators
        .iter()
        .filter_map(|&(id, next)| Some((schema.ids.relation(id)?, next)))
        .collect();
    if resumed.len() != generators.len() || !resumed.keys().copied().eq(generated(schema)) {
        return Err(damaged(GENERATORS));
    }
    engine.db.admit_restored_rows();
    let mut rows: BTreeMap<RelId, BTreeSet<Val>> = BTreeMap::new();
    for (r, c, name) in &data {
        let doc = std::str::from_utf8(&entries[name])
            .ok()
            .and_then(|text| read_document::<Val>(schema, text).ok())
            .filter(|d| d.header.as_ref() == Some(&header(e)) && d.relation == *r)
            .filter(|d| d.rows.iter().all(|row| ctor_of(row) == *c))
            .ok_or_else(|| damaged(name))?;
        rows.entry(*r).or_default().extend(doc.rows);
    }
    insert(db, schema, rows)?;
    let dormant = deployed[&grants]
        .iter()
        .filter(|g| dormant(schema, grants, g))
        .map(|g| dormant_grant(schema, grants, g).ok_or_else(|| damaged(IDS)))
        .collect::<Result<_, _>>()?;
    engine.replace_rows(deployed)?;
    for (r, next) in resumed {
        engine
            .db
            .resume_sequence(r, next)
            .map_err(|w| engine.write_error(super::TxnId(0), w))?;
    }
    Ok(dormant)
}

/// Returns the rows of `catalog::grants`, `replayed` with each of its `Log` rows given the id of
/// the row of `logged` that differs from it only in id and, when the replayed row is dormant, in
/// dormancy, or `None` when the two do not pair.
fn paired(
    schema: &Schema,
    grants: RelId,
    replayed: Vec<Val>,
    mut logged: BTreeSet<Val>,
) -> Option<BTreeSet<Val>> {
    let fields = layout::row(schema, grants, 0);
    let at = |name: &str| fields.iter().position(|(n, _)| n == name);
    let (id, sleeps) = (at("id")?, at("dormant")?);
    let with = |row: &Val, value: Option<&Val>| match row {
        Val::Ctor(c, fields) => {
            let mut fields = fields.to_vec();
            fields[id] = value.map_or(Val::Int(0), Val::clone);
            if value.is_none() {
                fields[sleeps] = Val::Int(0);
            }
            Some(Val::ctor(*c, fields))
        }
        _ => None,
    };
    let mut out = BTreeSet::new();
    for row in replayed {
        if deployment(schema, grants, &row) {
            out.insert(row);
            continue;
        }
        let pair = with(&row, None)?;
        let from = logged
            .iter()
            .find(|d| with(d, None).as_ref() == Some(&pair))
            .filter(|d| !dormant(schema, grants, d) || dormant(schema, grants, &row))?
            .clone();
        logged.remove(&from);
        out.insert(with(&row, field(schema, grants, &from, "id"))?);
    }
    logged.is_empty().then_some(out)
}

/// Returns whether `row` of `catalog::grants` is dormant.
fn dormant(schema: &Schema, grants: RelId, row: &Val) -> bool {
    matches!(
        field(schema, grants, row, "dormant").map(Val::truth),
        Some(Ok(true))
    )
}

/// Returns `rows`, rows of catalog relations of `from`, by the relations of `to` of those names.
fn on(from: &Schema, to: &Schema, rows: Rows) -> Rows {
    let named = |r: RelId| values::relation(to, &from.relation(r).name);
    rows.into_iter().map(|(r, v)| (named(r), v)).collect()
}

/// The catalog rows a restore parsed before it made its file, by relation of `schema`.
struct Parsed {
    schema: Schema,
    ids: Rows,
    logged: BTreeSet<Val>,
    deployed: Rows,
}

/// Returns the dormant `Log` row `row` of `catalog::grants` as a restore reports it.
fn dormant_grant(schema: &Schema, grants: RelId, row: &Val) -> Option<Dormant> {
    let text = |name: &str| match field(schema, grants, row, name)? {
        Val::Text(t) => Some(t.to_string()),
        _ => None,
    };
    let target = match field(schema, grants, row, "target")? {
        Val::Ctor(_, value) => match value.first() {
            Some(Val::Text(t)) => t.to_string(),
            _ => String::new(),
        },
        _ => return None,
    };
    let Val::Int(id) = field(schema, grants, row, "id")? else {
        return None;
    };
    Some(Dormant {
        id: *id,
        grantee: text("grantee")?,
        privilege: text("privilege")?,
        target,
    })
}

fn insert<R: Runtime>(
    db: &Database<R>,
    schema: &Schema,
    rows: BTreeMap<RelId, BTreeSet<Val>>,
) -> Result<(), Error> {
    let mut text = String::from("transact\n");
    let mut params = Vec::new();
    for (r, rows) in rows.into_iter().filter(|(_, rows)| !rows.is_empty()) {
        let rel = schema.relation(r);
        let name = format!("dump_rows_{}", params.len());
        let qualified = format!("{}::{}", schema.module(rel.module).name, rel.name);
        text.push_str(&format!("  insert {qualified} from ({name})\n"));
        params.push((name, Ty::Set(Box::new(rel.elem.clone())), Val::set(rows)));
    }
    if params.is_empty() {
        return Ok(());
    }
    text.push_str("  return {}\n");
    let params: Vec<(&str, Ty, Val)> = params
        .iter()
        .map(|(n, t, v)| (n.as_str(), t.clone(), v.clone()))
        .collect();
    db.query_text(&text, &params, &CallOptions::default())
        .map(|_| ())
}
