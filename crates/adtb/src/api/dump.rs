//! The command `dump`, and the restore of a dump into a new database file.

use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::schema::{Generator, Schema};
use adtb_lang::textual::{Form, Header, print, read_document};
use adtb_lang::types::{RelId, Ty};

use crate::exec::Val;
use crate::runtime::Runtime;
use crate::storage::{self, BackupError, DumpManifest, Envelope};
use crate::write::Txn;

use super::backup::{CATALOG_VERSION, RestorePoint, refused};
use super::bundle::EDITION;
use super::error::{FatalReason, RejectReason};
use super::{
    CallOptions, CreateOptions, Database, Engine, Error, Log, OpenError, OpenOptions, Position,
};

const LOG: &str = "log";
const IDS: &str = "ids";
const GENERATORS: &str = "generators";

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

fn keyed(schema: &Schema) -> Vec<RelId> {
    let catalog = schema
        .module_named("catalog")
        .expect("the engine declares module catalog");
    KEYED
        .iter()
        .map(|name| match schema.module(catalog).values.get(*name) {
            Some(adtb_lang::schema::ValueDef::Relation(r)) => *r,
            _ => panic!("the engine declares relation catalog::{name}"),
        })
        .collect()
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

/// Returns the `ids` entry: the rows of each relation of [`KEYED`].
fn ids_text(e: &Envelope, schema: &Schema, ids: &Ids) -> String {
    let mut out = header_line(e);
    for (&r, rows) in ids {
        out.push_str(&document(schema, r, rows, ""));
    }
    out
}

/// The rows of each relation of [`KEYED`].
type Ids = BTreeMap<RelId, BTreeSet<Val>>;

fn ids_of(
    schema: &Schema,
    mut rows: impl FnMut(RelId) -> Result<Vec<Val>, Error>,
) -> Result<Ids, Error> {
    keyed(schema)
        .into_iter()
        .map(|r| Ok((r, rows(r)?.into_iter().collect())))
        .collect()
}

/// Returns the rows an `ids` entry holds, by relation.
fn read_ids(schema: &Schema, text: &str, e: &Envelope) -> Option<Ids> {
    let rest = text.strip_prefix(&header_line(e))?;
    let mut sections: Vec<String> = Vec::new();
    for line in rest.split_inclusive('\n') {
        if line.starts_with("data ") {
            sections.push(String::new());
        }
        sections.last_mut()?.push_str(line);
    }
    let mut out = Ids::new();
    for section in &sections {
        let doc = read_document::<Val>(schema, section).ok()?;
        out.insert(doc.relation, doc.rows).is_none().then_some(())?;
    }
    Some(out)
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
        let ids = ids_of(schema, rows)?;
        let mut entries = vec![
            (LOG.to_owned(), log_text(&envelope, &self.source.log)),
            (IDS.to_owned(), ids_text(&envelope, schema, &ids)),
        ];
        let mut generators = header_line(&envelope);
        for r in dumped(schema).filter(|&r| sequenced(schema, r)) {
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
/// `options`. Returns the dump's manifest.
pub(super) fn restore_dump<R: Runtime>(
    rt: &R,
    source: &str,
    path: &str,
    to: RestorePoint,
    options: &OpenOptions,
) -> Result<DumpManifest, Error> {
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
    let ids = entry(IDS)?;
    let log_dir = options.storage(path).log_dir();
    let (aside, made) = storage::restore_aside(rt, path, &log_dir).map_err(refused)?;
    let aside_log = options.storage(&aside).log_dir();
    let creating = storage::creating(&aside);
    let opening = OpenOptions {
        create: Some(CreateOptions {
            after: Some(manifest.snapshot),
        }),
        ..options.clone()
    };
    let built = Database::open_in(rt.clone(), &aside, &Log::new(log), &opening)
        .map_err(|e| match e {
            OpenError::HostFnMissing { name } => Error::Fatal {
                reason: FatalReason::MissingFunction { function: name },
            },
            OpenError::Io(e) => Error::storage(format!("`{aside}`: {e}")),
            OpenError::MigrationRefused { .. } => damaged(LOG),
            e => Error::invalid(format!("the dumped log does not open: {e}")),
        })
        .and_then(|db| {
            let filled = fill(&db, &manifest, &entries, ids, &generators);
            let closed = db.close();
            filled.and(closed)
        })
        .and_then(|()| {
            storage::publish_database(rt, (&aside, &aside_log), path, &log_dir).map_err(refused)
        });
    if built.is_err() {
        storage::discard_aside(rt, &[&aside, &aside_log, &creating], &made);
    }
    built.map(|()| manifest)
}

/// Check the opened database against the dump's version, parts and `ids`, insert every dumped row
/// as one change set, and resume every generator.
fn fill<R: Runtime>(
    db: &Database<R>,
    manifest: &DumpManifest,
    entries: &BTreeMap<String, Vec<u8>>,
    ids: &str,
    generators: &[(u64, i64)],
) -> Result<(), Error> {
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
    let expected = |n: &str| [LOG, IDS, GENERATORS].contains(&n) || data.iter().any(|d| d.2 == n);
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
    if read_ids(schema, ids, e) != Some(ids_of(schema, read)?) {
        return Err(damaged(IDS));
    }
    let resumed: BTreeMap<RelId, i64> = generators
        .iter()
        .filter_map(|&(id, next)| Some((schema.ids.relation(id)?, next)))
        .collect();
    let sequences: Vec<RelId> = dumped(schema).filter(|&r| sequenced(schema, r)).collect();
    if resumed.len() != generators.len() || !resumed.keys().copied().eq(sequences) {
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
    for (r, next) in resumed {
        engine
            .db
            .resume_sequence(r, next)
            .map_err(|w| engine.write_error(super::TxnId(0), w))?;
    }
    Ok(())
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
