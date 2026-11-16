//! The command `dump`, and the restore of a dump into a new database file.

use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::ids::{CatRelId, engine_ctor as k, engine_rel, held_id};
use adtb_lang::kernel::{Build, Terms};
use adtb_lang::schema::{Committed, Generator, Schema};
use adtb_lang::textual::{Form, Header, print, read_document};
use adtb_lang::types::{RelId, Ty};

use crate::catalog::codec::{
    self, Codec, Ctor, Decode, Field, Fields, GrantKind, Relation, RoleGrant, RoleRow, relation,
};
use crate::catalog::{
    self, Deployed, Dormant, Fact, Mark, OWNER_ROLE, Origin, Rot, arrows, engine_relation,
};
use crate::exec::Val;
use crate::runtime::Runtime;
use crate::storage::{self, BackupError, DumpManifest, Envelope, Passphrase};
use crate::write::{self, Bounds, Local, Text, Txn};

use super::backup::{CATALOG_VERSION, RestorePoint, refused};
use super::bundle::{self, EDITION, Registered};
use super::error::{FatalReason, RejectReason};
use super::statements::Refusal;
use super::{
    CallOptions, CreateOptions, Database, Engine, Error, Log, OWNER_ID, OpenError, OpenOptions,
    Position,
};

#[cfg(test)]
#[path = "dump_tests.rs"]
mod tests;

const LOG: &str = "log";
const IDS: &str = "ids";
const GENERATORS: &str = "generators";
const SECURITY: &str = "security";
const SETTINGS: &str = "settings";
const AUDIT: &str = "audit";
const BUNDLES: &str = "bundles/";
/// The extension of a data entry in the Arrow form, which a restore reads.
const ARROWS: &str = "arrows";
/// The extension of a data entry in the text form, which a restore does not read.
const TEXT: &str = "txt";
/// The entry naming every bundle of a dump, with the owners of their families and their leases
/// latched as expired. A bundle's name holds a `-`, so no bundle's entry takes this name.
const INDEX: &str = "bundles/index";

/// The catalog relations holding the key of every catalog object but a constructor.
const KEYED: [CatRelId; 9] = [
    engine_rel::TYPES,
    engine_rel::FIELDS,
    engine_rel::RELATIONS,
    engine_rel::NAMED_SETS,
    engine_rel::INDEXES,
    engine_rel::CONSTRAINTS,
    engine_rel::FUNCTIONS,
    engine_rel::DEFAULTS,
    engine_rel::COMPUTED,
];
/// The catalog relations of the `security` entry, whose `grants` are its `Deployment` rows.
const SECURED: [CatRelId; 4] = [
    engine_rel::ROLES,
    engine_rel::CREDENTIALS,
    engine_rel::GRANTS,
    engine_rel::PEER_MAPPINGS,
];
/// The catalog relations whose keys the engine draws from a counter of its own.
const DRAWN: [CatRelId; 1] = [engine_rel::AUDIT];
/// The relations of [`DRAWN`] a dump holds the counter of only with its `security` entry.
const SECURED_DRAWN: [CatRelId; 2] = [engine_rel::ROLES, engine_rel::GRANTS];
/// The catalog relations of a bundle's entry, each holding the rows of that bundle alone.
const BUNDLED: [CatRelId; 4] = [
    engine_rel::BUNDLES,
    engine_rel::REGISTERED,
    engine_rel::FEEDS,
    engine_rel::FEED_MEMBERS,
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

/// Returns the relations a role must hold `read` on to dump besides those `print schema` reads.
pub(super) fn read_by_dump(schema: &Schema) -> Vec<RelId> {
    let catalog = [&[engine_rel::SETTINGS, engine_rel::AUDIT][..], &BUNDLED].concat();
    dumped(schema)
        .chain(catalog_relations(schema, &catalog))
        .collect()
}

fn catalog_relations(schema: &Schema, ids: &[CatRelId]) -> Vec<RelId> {
    ids.iter().map(|&id| engine_relation(schema, id)).collect()
}

/// Returns the relations whose next key a `generators` entry holds, in ascending order, those of
/// [`SECURED_DRAWN`] only when `secured`.
fn generated(schema: &Schema, secured: bool) -> Vec<RelId> {
    let mut out: Vec<RelId> = dumped(schema).filter(|&r| sequenced(schema, r)).collect();
    out.extend(catalog_relations(schema, &DRAWN));
    if secured {
        out.extend(catalog_relations(schema, &SECURED_DRAWN));
    }
    out.sort();
    out
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

fn rows_of<E>(
    schema: &Schema,
    ids: &[CatRelId],
    mut rows: impl FnMut(RelId) -> Result<Vec<Val>, E>,
) -> Result<Rows, E> {
    catalog_relations(schema, ids)
        .into_iter()
        .map(|r| Ok((r, rows(r)?.into_iter().collect())))
        .collect()
}

/// Returns the rows an entry of [`sections_text`] holds, once it holds a document for exactly
/// the relations `ids`.
fn read_sections(schema: &Schema, text: &str, e: &Envelope, ids: &[CatRelId]) -> Option<Rows> {
    sections(schema, text.strip_prefix(&header_line(e))?, ids)
}

/// Returns the rows of `rest`, once it holds a document for exactly the relations `ids`.
fn sections(schema: &Schema, rest: &str, ids: &[CatRelId]) -> Option<Rows> {
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
    let wanted: BTreeSet<RelId> = catalog_relations(schema, ids).into_iter().collect();
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

/// Returns the entry of each bundle of `rows`, rows of [`BUNDLED`], holding its registration mark
/// of `d` and its rows, then the [`INDEX`] entry.
fn bundle_entries(
    e: &Envelope,
    schema: &Schema,
    rows: &Rows,
    d: &Deployed,
) -> Result<Vec<(String, String)>, Rot> {
    let b = Bundled::new(schema);
    let (mut index, mut out) = (header_line(e), Vec::new());
    for (bundle, row) in decoded(schema, &b.bundles, rows)? {
        let (n, name) = (&bundle.id, &bundle.header.name);
        let mine = b.owned(schema, rows, *n, row)?;
        let mut text = header_line(e);
        if let Some(m) = d.registered.get(n) {
            let origin = match m.origin {
                Origin::Open => "open",
                Origin::Command => "command",
            };
            text.push_str(&format!(
                "mark {} {origin} {}\n{}\n",
                m.role,
                m.text.len(),
                m.text
            ));
        }
        for (&r, rows) in &mine {
            text.push_str(&document(schema, r, rows, ""));
        }
        index.push_str(&format!("bundle {name}\n"));
        if let Some(renewed) = d.expired.get(n) {
            index.push_str(&format!("expired {n} {renewed}\n"));
        }
        out.push((format!("{BUNDLES}{name}"), text));
    }
    for (family, role) in &d.families {
        index.push_str(&format!("family {role} {family}\n"));
    }
    out.push((INDEX.to_owned(), index));
    Ok(out)
}

/// Returns each row of `codec`'s relation in `rows` with its typed row.
fn decoded<'r, T: Decode>(
    schema: &Schema,
    codec: &Codec<T>,
    rows: &'r Rows,
) -> Result<Vec<(T, &'r Val)>, Rot> {
    rows.get(&codec.relation())
        .into_iter()
        .flatten()
        .map(|v| Ok((codec.decode_val(schema, v)?, v)))
        .collect()
}

/// The codecs of the relations of [`BUNDLED`].
struct Bundled {
    bundles: Codec<bundle::Row>,
    registered: Codec<Registered>,
    feeds: Codec<FeedRow>,
    members: Codec<FeedMemberRow>,
}

impl Bundled {
    fn new(schema: &Schema) -> Bundled {
        Bundled {
            bundles: Codec::new(schema),
            registered: Codec::new(schema),
            feeds: Codec::new(schema),
            members: Codec::new(schema),
        }
    }

    /// Returns the rows of `rows` that bundle `id`, whose row is `row`, holds, by relation.
    fn owned(&self, schema: &Schema, rows: &Rows, id: i64, row: &Val) -> Result<Rows, Rot> {
        let fed = decoded(schema, &self.feeds, rows)?;
        let fed: Vec<_> = fed.into_iter().filter(|(f, _)| f.bundle == id).collect();
        let feeds: BTreeSet<i64> = fed.iter().map(|(f, _)| f.id).collect();
        let calls = decoded(schema, &self.registered, rows)?;
        let members = decoded(schema, &self.members, rows)?;
        let calls = calls.into_iter().filter(|(c, _)| c.bundle() == id);
        let members = members.into_iter().filter(|(m, _)| feeds.contains(&m.feed));
        Ok(Rows::from([
            (self.bundles.relation(), BTreeSet::from([row.clone()])),
            (
                self.registered.relation(),
                calls.map(|(_, v)| v.clone()).collect(),
            ),
            (
                self.members.relation(),
                members.map(|(_, v)| v.clone()).collect(),
            ),
            (
                self.feeds.relation(),
                fed.into_iter().map(|(_, v)| v.clone()).collect(),
            ),
        ]))
    }
}

/// A row of `catalog::feeds`, the fields a dump reads.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FeedRow {
    id: i64,
    bundle: i64,
}

const FEED: Fields = &["id", "bundle", "name", "ordinal", "relation", "set"];
const FEED_ID: Field = codec::field(FEED, "id");
const FEED_BUNDLE: Field = codec::field(FEED, "bundle");

impl Relation for relation::Feeds {
    type Row = FeedRow;
}

impl Decode for FeedRow {
    type Relation = relation::Feeds;
    const CTORS: &'static [Ctor<FeedRow>] = &[Ctor::new(k::FEED, FEED, |r| {
        Ok(FeedRow {
            id: r.int(FEED_ID),
            bundle: r.int(FEED_BUNDLE),
        })
    })];
}

/// A row of `catalog::feed_members`, the field a dump reads.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FeedMemberRow {
    feed: i64,
}

const MEMBER: Fields = &["feed", "relation", "ctor"];
const MEMBER_FEED: Field = codec::field(MEMBER, "feed");

impl Relation for relation::FeedMembers {
    type Row = FeedMemberRow;
}

impl Decode for FeedMemberRow {
    type Relation = relation::FeedMembers;
    const CTORS: &'static [Ctor<FeedMemberRow>] = &[Ctor::new(k::FEED_MEMBER, MEMBER, |r| {
        Ok(FeedMemberRow {
            feed: r.int(MEMBER_FEED),
        })
    })];
}

/// Returns the rows of every bundle the [`INDEX`] entry names, by relation of `schema`, the
/// schema of a file holding no migration, and the deployment facts of the dump.
///
/// Fail with `DumpDamaged` naming the first entry under `bundles/` that does not decode, that the
/// index names and `entries` lacks, that the index does not name, or whose mark names a role
/// outside `roles`, then naming the index when it names a bundle twice, an owner outside `roles`,
/// or a family or latched bundle of no bundle it names.
fn read_bundles<'t>(
    schema: &Schema,
    e: &Envelope,
    roles: &BTreeSet<i64>,
    entry: impl Fn(&str) -> Result<&'t str, Error>,
    entries: impl Iterator<Item = &'t String>,
) -> Result<(Rows, Vec<Fact>), Error> {
    let (mut rows, mut facts, mut named, mut index) =
        (Rows::new(), Vec::new(), BTreeSet::new(), Vec::new());
    let text = entry(INDEX)?.strip_prefix(&header_line(e));
    for line in text.ok_or_else(|| damaged(INDEX))?.lines() {
        let number = |n: &str| n.parse::<i64>().ok();
        let fact = match line
            .split_once(' ')
            .map(|(k, rest)| (k, rest.split_once(' ')))
        {
            Some(("bundle", _)) => {
                if named.insert(format!("{BUNDLES}{}", &line["bundle ".len()..])) {
                    continue;
                }
                None
            }
            Some(("family", Some((role, family)))) => {
                number(role).map(|r| Fact::Family(family.to_owned(), r))
            }
            Some(("expired", Some((b, at)))) => number(b)
                .zip(number(at))
                .map(|(b, at)| Fact::Expired(b, at)),
            _ => None,
        };
        index.push(fact.ok_or_else(|| damaged(INDEX))?);
    }
    if let Some(stray) = entries
        .filter(|n| n.starts_with(BUNDLES))
        .find(|n| *n != INDEX && !named.contains(*n))
    {
        return Err(damaged(stray));
    }
    let (mut ids, mut families) = (BTreeSet::new(), BTreeSet::new());
    for name in &named {
        let text = entry(name)?.strip_prefix(&header_line(e));
        let (id, family, mark, mine) = text
            .and_then(|t| read_bundle(schema, t, &name[BUNDLES.len()..]))
            .filter(|(_, _, mark, _)| mark.as_ref().is_none_or(|m| roles.contains(&m.role)))
            .ok_or_else(|| damaged(name))?;
        ids.insert(id);
        families.insert(family);
        facts.extend(mark.map(|m| Fact::Registered(id, m)));
        for (r, held) in mine {
            rows.entry(r).or_default().extend(held);
        }
    }
    let known = |f: &Fact| match f {
        Fact::Family(family, role) => families.contains(family) && roles.contains(role),
        Fact::Expired(bundle, _) => ids.contains(bundle),
        _ => false,
    };
    if !index.iter().all(known) {
        return Err(damaged(INDEX));
    }
    facts.extend(index);
    Ok((rows, facts))
}

/// Returns the id, family, registration mark and rows of bundle `name` from `text`, its entry
/// after the header line, once its `catalog::bundles` holds `name`'s row alone and its other rows
/// are that bundle's.
fn read_bundle(
    schema: &Schema,
    text: &str,
    name: &str,
) -> Option<(i64, String, Option<Mark>, Rows)> {
    let (mut rest, mut mark) = (text, None);
    if let Some(after) = rest.strip_prefix("mark ") {
        let (line, after) = after.split_once('\n')?;
        let mut words = line.splitn(3, ' ');
        let role = words.next()?.parse().ok()?;
        let origin = match words.next()? {
            "open" => Origin::Open,
            "command" => Origin::Command,
            _ => return None,
        };
        let len = words.next()?.parse().ok()?;
        let text = after.get(..len)?.to_owned();
        rest = after.get(len..)?.strip_prefix('\n')?;
        mark = Some(Mark { role, origin, text });
    }
    let rows = sections(schema, rest, &BUNDLED)?;
    let b = Bundled::new(schema);
    let [(bundle, row)] = &decoded(schema, &b.bundles, &rows).ok()?[..] else {
        return None;
    };
    let owned = b.owned(schema, &rows, bundle.id, row).ok()?;
    let id = bundle.id;
    let family = bundle.header.family.clone();
    (bundle.header.name == name && owned == rows).then_some((id, family, mark, rows))
}

fn sequenced(schema: &Schema, r: RelId) -> bool {
    let rel = schema.relation(r);
    rel.generate == Some(Generator::Sequence) && rel.key.len() == 1
}

/// Returns the name of the entry holding the rows of constructor `ctor` of `r` in the form whose
/// extension is `form`.
fn data_name(schema: &Schema, r: RelId, ctor: u32, form: &str) -> String {
    let rel = schema.relation(r);
    let ctor = &schema.ty(rel.elem_def).ctors()[ctor as usize].name;
    let module = &schema.module(rel.module).name;
    format!("data/{module}::{}.{ctor}.{form}", rel.name)
}

fn ctor_of(v: &Val) -> u32 {
    match v {
        Val::Ctor(c, _) => *c,
        _ => panic!("a row of a relation is a value of a constructor of its sum: {v:?}"),
    }
}

impl<R: Runtime> Engine<R> {
    /// Fail with `Quarantined` naming the database when it is quarantined whole and a page the
    /// schema or the visibility of a row is read from fails its check against the retained log, or
    /// the retained log cannot settle an id in progress.
    fn admit_dump(&self) -> Result<(), Error> {
        let Some(refused) = self.refusal() else {
            return Ok(());
        };
        let Ok(pages) = self.catalog.read_from(&self.pager, self.db.txns()) else {
            return Err(refused);
        };
        let log = self.pager.read_log(true);
        let proven = self.db.txns().commit_log().unresolved(&self.pager).is_ok();
        if proven
            && pages
                .into_iter()
                .all(|pid| self.pager.check_page(pid, &log).is_empty())
        {
            Ok(())
        } else {
            Err(refused)
        }
    }

    /// Write the database as `tx` reads it to the dump directory `path`, with a `security` entry
    /// when `secured`, encrypted under `phrase` when given. Returns its manifest.
    pub(super) fn dump_to(
        &self,
        id: super::TxnId,
        tx: &Txn,
        path: &str,
        secured: bool,
        phrase: Option<&Passphrase>,
    ) -> Result<DumpManifest, Refusal> {
        let target = self.target(path).map_err(Refusal::own)?;
        self.admit_dump().map_err(Refusal::own)?;
        let envelope = self.envelope();
        let failed = |e| self.met(id, e);
        let broken = |r: Rot| failed(write::Error::from(write::Broken::CatalogError(r.into())));
        let rows = |r| self.db.relation_rows(tx, r).map_err(failed);
        let schema = &*self.schema;
        let grants = Codec::<RoleGrant>::new(schema);
        let (mut logged, mut given) = (BTreeSet::new(), BTreeSet::new());
        for g in rows(grants.relation())? {
            match grants.decode_val(schema, &g).map_err(broken)?.kind() {
                GrantKind::Log => logged.insert(g),
                GrantKind::Deployment => given.insert(g),
            };
        }
        let mut ids = rows_of(schema, &KEYED, rows)?;
        ids.insert(grants.relation(), logged);
        let mut sections = vec![(IDS, ids)];
        if secured {
            let mut security = rows_of(schema, &SECURED, rows)?;
            security.insert(grants.relation(), given);
            sections.push((SECURITY, security));
        }
        sections.push((SETTINGS, rows_of(schema, &[engine_rel::SETTINGS], rows)?));
        sections.push((AUDIT, rows_of(schema, &[engine_rel::AUDIT], rows)?));
        let mut entries = vec![(LOG.to_owned(), log_text(&envelope, &self.source.log))];
        for (name, rows) in sections {
            entries.push((name.to_owned(), sections_text(&envelope, schema, &rows)));
        }
        let mut generators = header_line(&envelope);
        for r in generated(schema, secured) {
            let next = self.db.sequence_next(r);
            let id = held_id(schema.ids().relation_id(r));
            generators.push_str(&format!("generator {id} {next}\n"));
        }
        entries.push((GENERATORS.to_owned(), generators));
        let deployed = self.db.deployed(tx, &self.deployment);
        let deployed = deployed.map_err(|e| self.met(id, e.into()))?;
        let bundled = rows_of(schema, &BUNDLED, rows)?;
        let bundles = bundle_entries(&envelope, schema, &bundled, &deployed).map_err(broken)?;
        entries.extend(bundles);
        let mut streams = Vec::new();
        for r in dumped(schema) {
            let mut by_ctor: BTreeMap<u32, BTreeSet<Val>> = BTreeMap::new();
            for row in self.db.relation_rows(tx, r).map_err(failed)? {
                by_ctor.entry(ctor_of(&row)).or_default().insert(row);
            }
            let ctors = schema.ty(schema.relation(r).elem_def).ctors().len();
            for c in 0..u32::try_from(ctors).expect("a sum has fewer than 2^32 constructors") {
                let rows = by_ctor.remove(&c).unwrap_or_default();
                let text = document(schema, r, &rows, &header_line(&envelope));
                entries.push((data_name(schema, r, c, TEXT), text));
                let stream = arrows::write_rows(schema, r, c, &rows, arrows::BATCH_LIMIT);
                streams.push((data_name(schema, r, c, ARROWS), stream));
            }
        }
        let entries: Vec<(String, Vec<u8>)> = entries
            .into_iter()
            .map(|(name, text)| (name, text.into_bytes()))
            .chain(streams)
            .collect();
        for r in dumped(schema) {
            self.db.admit_heaps(r).map_err(failed)?;
        }
        let snapshot = self.db.read_position(tx);
        storage::write_dump(&self.rt, &target, envelope, snapshot, &entries, phrase)
            .map_err(|e| Refusal::own(refused(e)))
    }
}

/// Restore the dump at `source`, encrypted under `phrase` when given, as the new database file
/// `path` of runtime `rt`, which opens with `options`. Returns the dump's manifest and each
/// dormant grant of the restored file.
pub(super) fn restore_dump<R: Runtime>(
    rt: &R,
    (source, phrase): (&str, Option<&Passphrase>),
    path: &str,
    to: RestorePoint,
    options: &OpenOptions,
) -> Result<(DumpManifest, Vec<Dormant>), Error> {
    let (manifest, entries) = storage::read_dump(rt, source, phrase).map_err(dump_refused)?;
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
    let section = |name: &str, ids: &[CatRelId]| {
        read_sections(&schema, entry(name)?, e, ids).ok_or_else(|| damaged(name))
    };
    let security = entries
        .get(SECURITY)
        .map(|_| security_rows(&schema, entry(SECURITY)?, e));
    let (security, ids) = security.transpose()?.unwrap_or_default();
    let roles = ids.into_iter().chain([OWNER_ID]).collect();
    let (bundled, facts) = read_bundles(&schema, e, &roles, entry, entries.keys())?;
    let deployed = section(SETTINGS, &[engine_rel::SETTINGS])?.into_iter();
    let deployed = deployed
        .chain(section(AUDIT, &[engine_rel::AUDIT])?)
        .chain(bundled)
        .collect();
    let mut ids = section(IDS, &[&KEYED[..], &[engine_rel::GRANTS]].concat())?;
    let grants = Codec::<RoleGrant>::new(&schema);
    let logged = ids.remove(&grants.relation()).unwrap_or_default();
    let held = dormant_grants(&schema, &grants, &logged).ok_or_else(|| damaged(IDS))?;
    let held = held
        .into_iter()
        .map(|d| (d.grantee, d.privilege, d.target))
        .collect();
    let parsed = Parsed {
        schema,
        ids,
        logged,
        deployed,
        facts,
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
        registrations: Vec::new(),
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
    let built = open(Log::default(), &opening)
        .and_then(|db| {
            let secured = db
                .engine
                .replace_rows(on(&parsed.schema, &db.engine.schema, security), &[]);
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
/// holding no migration, with the id of each role.
///
/// Fail with `DumpDamaged` naming `security` when a role or grant row is not one, its schema
/// owner has another name, or its grants hold a row of the log.
fn security_rows(schema: &Schema, text: &str, e: &Envelope) -> Result<(Rows, Vec<i64>), Error> {
    let rows = read_sections(schema, text, e, &SECURED).ok_or_else(|| damaged(SECURITY))?;
    let (roles, grants) = (
        Codec::<RoleRow>::new(schema),
        Codec::<RoleGrant>::new(schema),
    );
    let roles = decoded(schema, &roles, &rows).map_err(|_| damaged(SECURITY))?;
    let grants = decoded(schema, &grants, &rows).map_err(|_| damaged(SECURITY))?;
    let roles: Vec<(i64, String)> = roles
        .into_iter()
        .map(|(r, _)| {
            let (id, name, _) = r.into_parts();
            (id, name)
        })
        .collect();
    let owned = roles
        .iter()
        .any(|(id, name)| *id == OWNER_ID && name == OWNER_ROLE);
    if !owned
        || grants
            .iter()
            .any(|(g, _)| g.kind() != GrantKind::Deployment)
    {
        return Err(damaged(SECURITY));
    }
    Ok((rows, roles.into_iter().map(|(id, _)| id).collect()))
}

impl<R: Runtime> Engine<R> {
    /// Replace every row of each relation of `rows` by its rows and write `facts`, as one
    /// transaction.
    ///
    /// Precondition: each relation is one no migration writes.
    fn replace_rows(&self, rows: Rows, facts: &[Fact]) -> Result<(), Error> {
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
            let mut b = Build::new(&self.schema, &mut terms);
            let gone = b.rel(r, None);
            changes.push(b.delete(r, gone));
            let new = b.var(x);
            changes.push(b.insert(r, new));
        }
        let all = Build::new(&self.schema, &mut terms).par(changes);
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
        let x = tx
            .xid()
            .expect("a write transaction of the write path has an id");
        let written = self
            .db
            .statements(&tx, text, &mut Local::default(), Bounds::default())
            .map_err(|f| f.error)
            .and_then(|_| {
                let facts = self
                    .deployment
                    .insert(&self.pager, self.db.txns(), x, facts);
                facts.map_err(write::Error::from)
            });
        match written {
            Ok(()) => self.db.commit(tx).map(drop).map_err(failed),
            Err(e) => Err(write::either(self.db.abandon(tx, || failed(e), failed))),
        }
    }
}

/// Check the opened database against the dump's version, parts and `ids`, write every dumped row
/// unchecked, validate the whole database once, and resume every generator. Returns each dormant
/// grant it wrote.
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
                .map(move |c| (r, c, data_name(schema, r, c, ARROWS)))
        })
        .collect();
    if let Some((_, _, missing)) = data.iter().find(|(_, _, n)| !entries.contains_key(n)) {
        return Err(damaged(missing));
    }
    let mut allowed: BTreeSet<String> = [LOG, IDS, GENERATORS, SECURITY, SETTINGS, AUDIT]
        .map(str::to_owned)
        .into();
    for (r, c, name) in &data {
        allowed.extend([name.clone(), data_name(schema, *r, *c, TEXT)]);
    }
    let expected = |n: &str| allowed.contains(n) || n.starts_with(BUNDLES);
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
    let codec = Codec::<RoleGrant>::new(schema);
    let grants = codec.relation();
    let kept = paired(schema, &codec, read(grants)?, parsed.logged).ok_or_else(|| damaged(IDS))?;
    if on(&parsed.schema, schema, parsed.ids) != rows_of(schema, &KEYED, read)? {
        return Err(damaged(IDS));
    }
    let mut deployed = on(&parsed.schema, schema, parsed.deployed);
    deployed.insert(grants, kept);
    let resumed: BTreeMap<RelId, i64> = generators
        .iter()
        .filter_map(|&(id, next)| Some((schema.ids().relation_of(CatRelId::from_raw(id))?, next)))
        .collect();
    if resumed.len() != generators.len()
        || !resumed
            .keys()
            .copied()
            .eq(generated(schema, entries.contains_key(SECURITY)))
    {
        return Err(damaged(GENERATORS));
    }
    let failed = |w| engine.write_error(super::TxnId(0), w);
    let restoring = engine.db.admit_restored_rows();
    for (r, c, name) in &data {
        let rows = arrows::read_rows(schema, *r, *c, &entries[name]).map_err(|_| damaged(name))?;
        restoring
            .write(&BTreeMap::from([((*r, *c), rows.into_iter().collect())]))
            .map_err(failed)?;
    }
    restoring
        .validate(&dumped(schema).collect::<Vec<_>>())
        .map_err(failed)?;
    let dormant = dormant_grants(schema, &codec, &deployed[&grants]).ok_or_else(|| damaged(IDS))?;
    engine.replace_rows(deployed, &parsed.facts)?;
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
/// dormancy, or `None` when the two do not pair or `codec` refuses a row.
fn paired(
    schema: &Schema,
    codec: &Codec<RoleGrant>,
    replayed: Vec<Val>,
    logged: BTreeSet<Val>,
) -> Option<BTreeSet<Val>> {
    let read = |v: &Val| codec.decode_val(schema, v).ok();
    let mut logged: Vec<RoleGrant> = logged.iter().map(read).collect::<Option<_>>()?;
    let mut out = BTreeSet::new();
    for row in replayed {
        let g = read(&row)?;
        if g.kind() == GrantKind::Deployment {
            out.insert(row);
            continue;
        }
        let at = logged.iter().position(|d| d.pairs(&g))?;
        if logged[at].row().dormant && !g.row().dormant {
            return None;
        }
        let from = logged.remove(at);
        out.insert(RoleGrant::keyed(codec, &row, from.id()));
    }
    logged.is_empty().then_some(out)
}

/// Returns `rows`, rows of catalog relations of `from`, by the relations of `to` of those ids.
fn on(from: &Schema, to: &Schema, rows: Rows) -> Rows {
    let same = |r: RelId| engine_relation(to, held_id(from.ids().relation_id(r)));
    rows.into_iter().map(|(r, v)| (same(r), v)).collect()
}

/// The catalog rows a restore parsed before it made its file, by relation of `schema`.
struct Parsed {
    schema: Committed,
    ids: Rows,
    logged: BTreeSet<Val>,
    deployed: Rows,
    facts: Vec<Fact>,
}

/// Returns each dormant row of `rows`, rows of `catalog::grants`, as a restore reports it, or
/// `None` when `codec` refuses a row or a dormant row has no target.
fn dormant_grants(
    schema: &Schema,
    codec: &Codec<RoleGrant>,
    rows: &BTreeSet<Val>,
) -> Option<Vec<Dormant>> {
    let mut out = Vec::new();
    for v in rows {
        let g = codec.decode_val(schema, v).ok()?;
        let row = g.row();
        if row.dormant {
            out.push(Dormant {
                id: g.id(),
                grantee: row.grantee.to_owned(),
                privilege: row.privilege.to_owned(),
                target: row.target?.to_owned(),
            });
        }
    }
    Some(out)
}
