//! Registered queries and transactions: checked and normalised when the database opens, called
//! by ordinal with arguments, each for the principal of its call.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Weak};
use std::time::SystemTime;

use adtb_lang::ast::{BundleItem, Import, OperatorKind};
use adtb_lang::capability::RootKind;
use adtb_lang::check::{Bundles, Compiled};
use adtb_lang::evolve::{DeployedGrant, LogGrant, Source};
use adtb_lang::kernel::{Terms, Tm, Var};
use adtb_lang::norm::{self, NormalProgram, NormalQuery};
use adtb_lang::parse::{Options, parse_bundle, parse_bundle_body, parse_migration};
use adtb_lang::permission::{self, EffectiveGrants, Mode, Requirement, Requirements};
use adtb_lang::schema::Committed as CommittedSchema;
use adtb_lang::schema::Schema;
use adtb_lang::signature;
use adtb_lang::translate::{
    Pinned, Refused, Untranslatable, held_other_code, host_conflict, operator_text,
};
use adtb_lang::types::Ty;

use crate::catalog::{self, Catalog, VersionHash};
use crate::exec::{Priority, Val};
use crate::runtime::Runtime;

use super::admitted::{Authority, CALL_BYTES, Queued};
use super::arrow::{self, ArrowStream};
use super::bundle::{self, CallRow, Callable, EDITION, Header, Passed, Stored};
use super::cause::{Broken, internal};
use super::error::{Error, RejectReason};
use super::pinned;
use super::reads::Reads;
use super::server::{holds_monitor, monitored};
use super::settings::Settings;
use super::stated;
use super::text::{DynRows, Normal, Prepared, rows};
use super::txn::{AdHocTransaction, Committed, TxnCallOptions};
use super::{CallOptions, Database, Engine, OWNER_ID, OpenError};

mod check;

use check::checked;

/// The queries, transactions and imported schema functions one unit registers: a bundle file
/// with its header, or the imports and items of one without.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    name: String,
    text: String,
    header: Option<Header>,
}

impl Registration {
    /// Returns the registration `name` of `text`, the imports and items of a bundle file with no
    /// header, checked at the version open leaves the file at.
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Registration {
        Registration {
            name: name.into(),
            text: text.into(),
            header: None,
        }
    }

    /// Returns the registration of the bundle file `text`, named by its header and checked at
    /// the version it pins.
    ///
    /// Fail with `Invalid` when `text` does not parse as a bundle file, and when its header
    /// states a clause this engine does not run.
    pub fn bundle(text: impl Into<String>) -> Result<Registration, Error> {
        let text = text.into();
        let file = parse_bundle(&text, Options::default())
            .ok()
            .map_err(|ds| Error::invalid(joined(ds.iter().map(|d| d.message.as_str()))))?;
        let header = Header::of(&file.header)?;
        Ok(Registration {
            name: header.name.clone(),
            text,
            header: Some(header),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn header(&self) -> Option<&Header> {
        self.header.as_ref()
    }

    /// Returns the imports and items of the registration's text.
    fn body(&self) -> Result<(Vec<Import>, Vec<BundleItem>), Error> {
        let failed = |ds: Vec<adtb_lang::span::Diag>| {
            Error::invalid(joined(ds.iter().map(|d| d.message.as_str())))
        };
        if self.header.is_some() {
            let file = parse_bundle(&self.text, Options::default())
                .ok()
                .map_err(failed)?;
            Ok((file.imports, file.items))
        } else {
            parse_bundle_body(&self.text, Options::default())
                .ok()
                .map_err(failed)
        }
    }
}

/// What a call of a registered query, transaction or function returned outside an interactive
/// transaction.
#[derive(Debug)]
pub enum Called {
    /// The elements of a set-valued result, else the one value.
    Rows(DynRows),
    /// The one result of a registered transaction, with its commit position.
    Committed(Committed<DynRows>),
}

/// What a call inside an interactive transaction returned.
#[derive(Debug)]
pub enum InTxn {
    Rows(DynRows),
    /// The one result of a registered transaction, committed with the transaction.
    Pending(DynRows),
}

/// One callable of a registration, normalised at the schema version it was registered at.
#[derive(Debug)]
struct Entry {
    name: String,
    kind: Callable,
    terms: Arc<Terms>,
    body: Tm,
    /// The variable each argument binds, in parameter order; `None` for an argument the body
    /// does not read.
    params: Vec<Option<Var>>,
    /// The type of each declared parameter, in order.
    types: Vec<Ty>,
    /// The name of each declared parameter, in order.
    names: Vec<String>,
    /// The result type as declared.
    result: Ty,
    /// The variable of the unit parameter a body with no declared parameter takes.
    unit: Option<Var>,
    writes: bool,
    retry: Option<u64>,
    /// Its row of `catalog::registered`, its ordinal from 1.
    row: CallRow,
    /// Its translation to the version the database is at, for a registration checked at an
    /// earlier one.
    served: Option<Served>,
    /// The id of the schema function an imported function calls.
    function: Option<u64>,
    /// The `module::name` of the schema function an imported function calls.
    function_name: Option<String>,
}

/// A callable translated to the version the database is at, its variables as [`Entry`]'s.
#[derive(Debug)]
struct Served {
    /// The schema it runs over and the program of its functions.
    compiled: Compiled,
    terms: Arc<Terms>,
    body: Tm,
    params: Vec<Option<Var>>,
    unit: Option<Var>,
}

/// A registration whose every callable checked.
#[derive(Debug)]
pub(super) struct Checked {
    name: String,
    /// The schema its callables run over, the version's with the registration's functions, and
    /// its program.
    compiled: Compiled,
    /// The text of the registration's file.
    text: String,
    entries: Vec<Entry>,
    /// What the callables read by occurrence, before normalisation.
    source_reads: Reads,
    /// The header of a bundle file's registration.
    header: Option<Header>,
}

impl Checked {
    fn schema(&self) -> &Arc<CommittedSchema> {
        self.compiled.shared_schema()
    }

    fn entry(&self, ordinal: u32) -> Option<&Entry> {
        let i = usize::try_from(ordinal).ok()?.checked_sub(1)?;
        self.entries.get(i)
    }

    /// Returns the header and calls of a bundle's registration.
    pub(super) fn bundle(&self) -> Option<(&Header, Vec<CallRow>)> {
        let calls = self.entries.iter().map(|e| e.row.clone()).collect();
        self.header.as_ref().map(|h| (h, calls))
    }

    /// Returns a bundle's registration as one that passed every check.
    pub(super) fn passed(&self) -> Option<Passed<'_>> {
        let (header, calls) = self.bundle()?;
        Some(Passed {
            header,
            calls,
            text: &self.text,
        })
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    fn declared(&self, query: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.kind != Callable::Fn && e.name == query)
    }

    /// Returns the identity of the signature of the query or transaction `query`.
    pub(super) fn call_ids(&self, query: &str) -> Option<signature::Ids> {
        let e = self.declared(query)?;
        Some(signature::Ids::of(self.schema(), &e.types, &e.result))
    }

    /// Returns the version the bundle pins and the family call target of its query or transaction
    /// `query`, whose types are as they stand at that version.
    pub(super) fn call_target(&self, query: &str) -> Option<(u64, String)> {
        let e = self.declared(query)?;
        let h = self.header.as_ref()?;
        let params: Vec<(String, Ty)> = e.names.iter().cloned().zip(e.types.clone()).collect();
        let target =
            signature::family_target(self.schema(), &h.family, query, h.pins, &params, &e.result);
        Some((h.pins, target))
    }

    /// Returns the catalog reader of a bundle's registration, at the version it pins.
    fn reader(&self) -> Option<catalog::Reader> {
        let h = self.header.as_ref()?;
        let mut reads = self.source_reads.clone();
        for e in &self.entries {
            reads.add(self.schema(), &e.terms, e.body);
        }
        Some(reads.reader(self.schema(), &h.name, h.pins, h.hides_new))
    }

    /// Returns `rows`, the result of a call of `entry`, encoded at the version the registration
    /// is checked at.
    fn encoded(&self, entry: &Entry, rows: DynRows) -> Result<DynRows, Error> {
        let Some(served) = &entry.served else {
            return Ok(rows);
        };
        let map = pinned::Versions::new(self.schema(), served.schema());
        let ty = match entry.terms.ty(entry.body) {
            Ty::Set(t) | Ty::NonEmpty(t) => t.as_ref().clone(),
            t => t.clone(),
        };
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for v in rows {
            let v = pinned::back(&map, &ty, &v?);
            if seen.insert(v.clone()) {
                out.push(v);
            }
        }
        Ok(super::text::rows(self.schema(), ty, out))
    }

    /// Record the queries and transactions of a bundle whose calls run in `bundles`.
    pub(super) fn explained(&self, bundles: &mut Bundles) {
        let translated = self.entries.iter().any(|e| e.served.is_some());
        let Some(h) = self.header.as_ref().filter(|_| !translated) else {
            return;
        };
        for e in self.entries.iter().filter(|e| e.kind != Callable::Fn) {
            bundles.insert(&h.family, &h.version, &e.name, e.types.clone());
        }
    }
}

/// What the owners of bundles lack among one state of the grants. Each program normalises once
/// and each role's grants are resolved once per schema, whatever the number of bundles.
pub(super) struct Lacking<'a> {
    log: &'a [LogGrant],
    deployed: &'a [DeployedGrant],
    held: Vec<Held>,
}

/// What every callable over one schema and program shares.
struct Held {
    compiled: Compiled,
    normal: NormalProgram,
    roles: BTreeMap<String, EffectiveGrants>,
}

impl<'a> Lacking<'a> {
    pub(super) fn new(log: &'a [LogGrant], deployed: &'a [DeployedGrant]) -> Self {
        Lacking {
            log,
            deployed,
            held: Vec::new(),
        }
    }

    /// Returns what `role` lacks of what the queries and transactions of `c` use as they run:
    /// each translation where there is one. An imported schema function needs nothing of `role`.
    /// Each is a line naming the callable, what it does and the privilege, in ordinal order.
    pub(super) fn of(&mut self, c: &Checked, role: &str) -> Result<Vec<String>, Error> {
        let runs = c.entries.iter().map(|e| match &e.served {
            Some(s) => (e, s.run()),
            None => (
                e,
                Run {
                    compiled: &c.compiled,
                    terms: &e.terms,
                    body: e.body,
                    params: &e.params,
                    unit: e.unit,
                },
            ),
        });
        self.of_runs(runs, role)
    }

    /// Returns what [`Lacking::of`] does, for the callables `runs` runs.
    fn of_runs<'r>(
        &mut self,
        runs: impl Iterator<Item = (&'r Entry, Run<'r>)>,
        role: &str,
    ) -> Result<Vec<String>, Error> {
        let mut out = Vec::new();
        for (e, run) in runs.filter(|(e, _)| e.kind != Callable::Fn) {
            let at = self.held_at(&run)?;
            let needs = requirements(&run, &self.held[at].normal)?;
            let (log, deployed) = (self.log, self.deployed);
            let held = self.held[at]
                .roles
                .entry(role.to_owned())
                .or_insert_with(|| {
                    EffectiveGrants::of(run.compiled.committed(), log, deployed, role)
                });
            let monitor = holds_monitor(deployed, role);
            let missing = needs.missing(held, Mode::Registration).into_iter();
            for r in missing.filter(|r| !(monitor && monitored(run.compiled.committed(), r))) {
                let (does, privilege) = r.text(run.compiled.committed());
                out.push(if matches!(r, Requirement::ReadFiles) {
                    format!("{} {does}; {role} lacks {privilege}", e.name)
                } else {
                    format!(
                        "{} {does}; {role} lacks {privilege} with grant option",
                        e.name
                    )
                });
            }
        }
        Ok(out)
    }

    /// Returns where `self.held` holds the schema and program of `run`.
    fn held_at(&mut self, run: &Run<'_>) -> Result<usize, Error> {
        if let Some(at) = self.held.iter().position(|h| h.compiled.same(run.compiled)) {
            return Ok(at);
        }
        let normal = norm::program(run.compiled)
            .map_err(|e| Error::not_normal(e, Broken::RegistrationNotNormal))?;
        self.held.push(Held {
            compiled: run.compiled.clone(),
            normal,
            roles: BTreeMap::new(),
        });
        Ok(self.held.len() - 1)
    }
}

/// What a callable runs: its normal term over its parameters and the schema and program it runs
/// over.
struct Run<'a> {
    compiled: &'a Compiled,
    terms: &'a Terms,
    body: Tm,
    params: &'a [Option<Var>],
    unit: Option<Var>,
}

impl Served {
    fn schema(&self) -> &Arc<CommittedSchema> {
        self.compiled.shared_schema()
    }

    fn run(&self) -> Run<'_> {
        Run {
            compiled: &self.compiled,
            terms: &self.terms,
            body: self.body,
            params: &self.params,
            unit: self.unit,
        }
    }
}

fn requirements(run: &Run<'_>, program: &NormalProgram) -> Result<Requirements, Error> {
    let params: Vec<Var> = run
        .params
        .iter()
        .flatten()
        .copied()
        .chain(run.unit)
        .collect();
    permission::normal(
        run.compiled.committed(),
        program,
        run.terms,
        run.body,
        &params,
    )
    .map_err(|e| Error::unanalysed(e, Broken::NoRequirements))
}

/// Refuse a bundle of `checked`, recorded in `stored` with an owner other than the schema
/// owner, whose owner lacks a grant among those of `source` and `deployed` it uses.
pub(super) fn definers(
    checked: &[Checked],
    stored: &Stored,
    source: &Source,
    deployed: &[DeployedGrant],
    roles: &BTreeMap<i64, String>,
) -> Result<(), OpenError> {
    let mut lacking = Lacking::new(&source.grants, deployed);
    for c in checked {
        let refused = |reason| OpenError::RegistrationRefused {
            bundle: c.name.clone(),
            reason,
        };
        let Some(owner) = definer(stored, &c.name, roles).map_err(refused)? else {
            continue;
        };
        let lacks = lacking.of(c, &owner).map_err(refused)?;
        if !lacks.is_empty() {
            return Err(refused(Error::rejected(RejectReason::Denied {
                detail: lacks.join("\n"),
            })));
        }
    }
    Ok(())
}

/// Returns the name of the role bundle `name` of `stored` runs as, by its id in `roles`; `None`
/// for the schema owner or a bundle `stored` does not hold.
pub(super) fn definer(
    stored: &Stored,
    name: &str,
    roles: &BTreeMap<i64, String>,
) -> Result<Option<String>, Error> {
    let Some(row) = stored.bundles.get(name) else {
        return Ok(None);
    };
    let id = stored.owner_of(row);
    if id == OWNER_ID {
        return Ok(None);
    }
    role_name(roles, id, &format!("bundle {name}")).map(Some)
}

/// Returns the name `roles` gives the role of id `id`, which owns `owned`.
pub(super) fn role_name(
    roles: &BTreeMap<i64, String>,
    id: i64,
    owned: &str,
) -> Result<String, Error> {
    roles.get(&id).cloned().ok_or_else(|| {
        Error::internal(Broken::NoRole {
            id,
            owned: owned.to_owned(),
        })
    })
}

/// The schema a registration checks against and the program of its functions.
pub(super) struct Over<'a> {
    pub(super) source: &'a Source,
    pub(super) compiled: &'a Compiled,
    /// The hash of the version `source` is at.
    hash: &'a VersionHash,
}

impl<'a> Over<'a> {
    pub(super) fn new(
        source: &'a Source,
        compiled: &'a Compiled,
        hash: &'a VersionHash,
    ) -> Over<'a> {
        Over {
            source,
            compiled,
            hash,
        }
    }
}

/// Returns `compiled` with its schema committed, the schema of a settled source.
pub(super) fn settled(compiled: Compiled<Schema>) -> Result<Compiled, RejectReason> {
    compiled
        .commit()
        .ok_or_else(|| internal(Broken::Catalog(catalog::Broken::Unsettled)))
}

/// A version a bundle pins: its source, schema and program.
struct Pin {
    source: Source,
    compiled: Compiled,
    hash: VersionHash,
}

impl Pin {
    fn over(&self) -> Over<'_> {
        Over {
            source: &self.source,
            compiled: &self.compiled,
            hash: &self.hash,
        }
    }
}

/// Check and normalise every registration of `regs`, all or none: one with no header over
/// `over`, the version the database is at, and a bundle's at the version it pins, refusing one
/// `stored` holds otherwise or that would read a constructor enabled since its pin, at `now`
/// under the file settings `s`.
pub(super) fn register(
    over: &Over<'_>,
    regs: &[Registration],
    stored: &Stored,
    now: i64,
    s: &Settings,
) -> Result<Vec<Checked>, OpenError> {
    let mut names = BTreeSet::new();
    let mut pins = Pins::default();
    let mut out = Vec::with_capacity(regs.len());
    for r in regs {
        let refused = |reason: Error| OpenError::RegistrationRefused {
            bundle: r.name.clone(),
            reason,
        };
        if !names.insert(r.name.as_str()) {
            return Err(refused(Error::invalid(format!(
                "`{}` is registered twice",
                r.name
            ))));
        }
        out.push(registration(over, r, stored, now, s, &mut pins).map_err(refused)?);
    }
    Ok(out)
}

/// Check and normalise registration `r` as [`register`] does.
pub(super) fn registration(
    over: &Over<'_>,
    r: &Registration,
    stored: &Stored,
    now: i64,
    s: &Settings,
    pins: &mut Pins,
) -> Result<Checked, Error> {
    let Some(h) = &r.header else {
        let c = checked(over, r)?;
        pinned_within(&c, over)?;
        return Ok(c);
    };
    let at_pin = pinned(over, h, pins)?;
    let mut c = match &at_pin {
        Some(pin) => checked(&pin.over(), r),
        None => checked(over, r),
    }?;
    let (_, calls) = c.bundle().expect("a bundle's registration has a header");
    bundle::admitted(stored, h, &calls)?;
    if let Some(pin) = &at_pin {
        if bundle::going_live(stored, h, now, s)
            && let Some(f) = held_other_code(&pin.source, over.source)
                .map_err(|ds| Error::internal(Broken::HostCode(ds)))?
        {
            let u = Untranslatable {
                operator: None,
                reason: format!("its version holds another code of host or Wasm function `{f}`"),
            };
            let first = c.entries.first().map_or("", |e| e.name.as_str());
            return Err(untranslatable(stored, h, now, s, &c.name, first, &u));
        }
        let served = translations(&c, &pin.source, over.compiled, over.source)
            .map_err(|b| Error::internal(*b))?
            .map_err(|(query, u)| untranslatable(stored, h, now, s, &c.name, &query, &u))?;
        for (e, s) in c.entries.iter_mut().zip(served) {
            e.served = Some(s);
        }
    }
    pinned_within(&c, over)?;
    if let Some(ctor) = stale(over, &c)? {
        return Err(bundle::stale(stored, h, now, s, ctor));
    }
    Ok(c)
}

/// Returns the refusal of registering `h`, whose callable `query` of bundle `name` has no
/// translation for `u`, at `now` under the file settings `s`: `BundleExpired` for a bundle whose
/// lease ran out or that is superseded, else `Untranslatable`.
fn untranslatable(
    stored: &Stored,
    h: &Header,
    now: i64,
    s: &Settings,
    name: &str,
    query: &str,
    u: &Untranslatable,
) -> Error {
    let query = format!("{name}::{query}");
    if bundle::lapsed(stored, h, now, s) {
        return Error::rejected(RejectReason::BundleExpired {
            bundle: h.name.clone(),
            query,
        });
    }
    Error::rejected(RejectReason::Untranslatable {
        query,
        detail: u.to_string(),
    })
}

/// Refuse a callable of `c` stating a version past the log `over` reads or with another hash.
fn pinned_within(c: &Checked, over: &Over<'_>) -> Result<(), Error> {
    for e in &c.entries {
        stated::held(&e.terms, over.hash)?;
    }
    Ok(())
}

/// Refuse an open whose bundles pin a version before the file's and would read a constructor
/// enabled since. A bundle that does not check at its pin is left for registration to refuse.
pub(super) fn downgrade(catalog: &Catalog, regs: &[Registration]) -> Result<(), OpenError> {
    let hash = catalog.version_hash();
    let now = Over {
        source: catalog.source(),
        compiled: catalog.compiled(),
        hash: &hash,
    };
    let version = catalog.source().version();
    for r in regs {
        let Some(h) = r.header.as_ref().filter(|h| h.pins < version) else {
            continue;
        };
        let Ok(Some(pin)) = pinned(&now, h, &mut Pins::default()) else {
            continue;
        };
        let Ok(c) = checked(&pin.over(), r) else {
            continue;
        };
        if let Ok(Some(ctor)) = stale(&now, &c) {
            return Err(OpenError::DowngradeRefused {
                cause: format!(
                    "bundle {}, pinned at version {}, reads {ctor}, enabled since",
                    h.name, h.pins
                ),
            });
        }
    }
    Ok(())
}

/// The versions bundles pin, each checked once. Bundles at one version share its schema and
/// program.
#[derive(Default)]
pub(super) struct Pins(BTreeMap<u64, Arc<Pin>>);

/// Returns the version `h` pins, `None` for the version `now` is at. Refuses an edition the
/// engine does not implement, a catalog pin, and a pinned version the log does not hold or holds
/// with another hash.
fn pinned(now: &Over<'_>, h: &Header, pins: &mut Pins) -> Result<Option<Arc<Pin>>, Error> {
    if h.edition != EDITION {
        return Err(Error::rejected(RejectReason::UnknownEdition {
            edition: h.edition.clone(),
        }));
    }
    if let Some(v) = h.catalog {
        return Err(Error::invalid(format!(
            "bundle {} pins catalog version {v}, and this engine's catalog has no versions",
            h.name
        )));
    }
    let at = now.source.version();
    let hashed = |over: &Over<'_>| {
        let hash = over
            .hash
            .get()
            .map_err(|e| Error::internal(Broken::CatalogError(e)))?;
        let hash = stated::hex(&hash);
        if hash == h.hash {
            return Ok(());
        }
        let what = format!("bundle {}", h.name);
        Err(stated::differs(&what, h.pins, &h.hash, &hash))
    };
    if h.pins == at {
        return hashed(now).map(|()| None);
    }
    if let Some(pin) = pins.0.get(&h.pins) {
        return hashed(&pin.over()).map(|()| Some(pin.clone()));
    }
    let source = catalog::source_at(now.source, h.pins)
        .ok_or_else(|| stated::unheld(&format!("bundle {}", h.name), h.pins, at))?;
    let compiled = source
        .check()
        .map_err(|ds| Error::invalid(joined(ds.iter().map(|d| d.diag.message.as_str()))))?;
    let pin = Arc::new(Pin {
        source,
        compiled: settled(compiled).map_err(Error::rejected)?,
        hash: now.hash.upto(h.pins),
    });
    pins.0.insert(h.pins, pin.clone());
    hashed(&pin.over())?;
    Ok(Some(pin))
}

/// Returns the constructor the bundle `c` would read that is enabled now and absent at its pin.
fn stale(now: &Over<'_>, c: &Checked) -> Result<Option<String>, Error> {
    let Some(reader) = c.reader() else {
        return Ok(None);
    };
    catalog::stale(now.source, now.compiled.committed(), &reader).map_err(catalog_error)
}

fn catalog_error(e: catalog::Error) -> Error {
    match e {
        catalog::Error::Refused(why) | catalog::Error::Unaccepted(_, why) => {
            Error::invalid(why.join("\n"))
        }
        catalog::Error::PastLimits(_) => Error::too_large(),
        other => Error::internal(Broken::CatalogError(other)),
    }
}

/// A live bundle, checked at the version it pins, with the source of that version and its
/// owner when that is not the schema owner.
pub(super) struct Live {
    checked: Checked,
    source: Source,
    definer: Option<String>,
}

/// Register as a reader of `catalog` each bundle of `regs` that `live` names, at the version it
/// pins, its owner as `stored` records it. Returns those bundles.
pub(super) fn readers(
    catalog: &mut Catalog,
    regs: &[Registration],
    live: &BTreeSet<String>,
    stored: &Stored,
    roles: &BTreeMap<i64, String>,
) -> Result<Vec<Live>, OpenError> {
    let mut out = Vec::new();
    let mut pins = Pins::default();
    let compiled = catalog.compiled().clone();
    for r in regs {
        let Some(h) = r.header.as_ref().filter(|h| live.contains(&h.name)) else {
            continue;
        };
        let refused = |reason: Error| OpenError::RegistrationRefused {
            bundle: r.name.clone(),
            reason,
        };
        let hash = catalog.version_hash();
        let now = Over {
            source: catalog.source(),
            compiled: &compiled,
            hash: &hash,
        };
        let (c, source) = match pinned(&now, h, &mut pins).map_err(refused)? {
            Some(pin) => (checked(&pin.over(), r), pin.source.clone()),
            None => (checked(&now, r), catalog.source().clone()),
        };
        let c = c.map_err(refused)?;
        let reader = c.reader().expect("a bundle's registration has a header");
        catalog
            .register(reader)
            .map_err(|e| refused(catalog_error(e)))?;
        let definer = definer(stored, &h.name, roles).map_err(refused)?;
        out.push(Live {
            checked: c,
            source,
            definer,
        });
    }
    Ok(out)
}

/// Refuse migration `text`, applied to the version `catalog` is at, when it changes a host
/// function's code from what a bundle of `live` pins, when a query or transaction of a bundle of
/// `live` has no translation to the version it gives, or when the owner of a
/// bundle, other than the schema owner, lacks after it, among its grants and `deployed`, a grant
/// the translation uses. The refusal names every blocking bundle, in name order, and the operator
/// the first bundle of `live` that blocks it is blocked by: the one its translation fails across, or the first
/// `revoke` from its owner, or else the migration's first operator. A migration that does not
/// apply or check is left for the catalog to refuse; fail with an internal error
/// when the migration model fails on it.
pub(super) fn recheck(
    catalog: &Catalog,
    text: &str,
    live: &[Live],
    deployed: &[DeployedGrant],
) -> Result<(), RejectReason> {
    if live.is_empty() {
        return Ok(());
    }
    let Ok(file) = parse_migration(text, Options::default()).ok() else {
        return Ok(());
    };
    let next = match catalog.source().migrate(&file, text) {
        Ok(next) => next,
        Err(f) => {
            return catalog::model_fault(&f).map_or(Ok(()), |b| Err(internal(Broken::Catalog(b))));
        }
    };
    let Ok(compiled) = next.check() else {
        return Ok(());
    };
    let compiled = settled(compiled)?;
    let mut lacking = Lacking::new(&next.grants, deployed);
    let mut operator = None;
    let mut blocked: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for l in live {
        let c = &l.checked;
        let pins = c.header.as_ref().map_or(0, |h| h.pins);
        let lines = blocked.entry(&c.name).or_default();
        let conflict = host_conflict(catalog.source(), &l.source, &next)
            .map_err(|ds| internal(Broken::HostCode(ds)))?;
        if let Some((op, f)) = conflict {
            operator.get_or_insert(op);
            lines.push(format!(
                "bundle {} pins {pins}: its version holds another code of host or Wasm function `{f}`",
                c.name
            ));
            continue;
        }
        let served = match translations(c, &l.source, &compiled, &next).map_err(|b| internal(*b))? {
            Ok(served) => served,
            Err((query, u)) => {
                if let Some(op) = &u.operator {
                    operator.get_or_insert_with(|| op.clone());
                }
                lines.push(format!(
                    "bundle {} pins {pins}: `{query}` has no translation: {}",
                    c.name, u.reason
                ));
                continue;
            }
        };
        let Some(owner) = &l.definer else {
            continue;
        };
        let runs = c.entries.iter().zip(served.iter().map(Served::run));
        let lacks = lacking.of_runs(runs, owner).map_err(|e| match e {
            Error::Rejected { reason } => reason,
            other => internal(Broken::Lacking(other)),
        })?;
        if !lacks.is_empty() {
            let revoking = file.operators.iter().find(|op| match &op.kind {
                OperatorKind::Revoke(r) => r.from.text == *owner,
                _ => false,
            });
            if let Some(op) = revoking.or(file.operators.first()) {
                operator.get_or_insert_with(|| operator_text(text, op));
            }
        }
        lines.extend(
            lacks
                .into_iter()
                .map(|lack| format!("bundle {} pins {pins}: {lack}", c.name)),
        );
    }
    let lines: Vec<String> = blocked.into_values().flatten().collect();
    if lines.is_empty() {
        return Ok(());
    }
    Err(RejectReason::SchemaRefused {
        operator: operator.unwrap_or_else(|| {
            file.operators
                .first()
                .map(|op| operator_text(text, op))
                .unwrap_or_default()
        }),
        detail: lines.join("\n"),
    })
}

/// The translation of each callable, or the name of the first that has none and why.
type Translated = Result<Vec<Served>, (String, Untranslatable)>;

/// Returns the translation of each callable of `c`, checked at the version of `pin`, to `now`,
/// the schema of `now_source` and its program, or the name of the first callable that has none,
/// and why. Fail when a stored body or a translation breaks an invariant of the kernel.
fn translations(
    c: &Checked,
    pin: &Source,
    now: &Compiled,
    now_source: &Source,
) -> Result<Translated, Box<Broken>> {
    let Some(first) = c.entries.first() else {
        return Ok(Ok(Vec::new()));
    };
    let hides = c.header.as_ref().is_some_and(|h| h.hides_new);
    let p = match Pinned::new(c.schema(), pin, now, now_source, hides) {
        Ok(p) => p,
        Err(Refused::Untranslatable(u)) => return Ok(Err((first.name.clone(), u))),
        Err(Refused::Broken(n)) => return Err(Box::new(Broken::TranslationNotNormal(n))),
        Err(Refused::Model(why)) => {
            return Err(Box::new(Broken::Catalog(catalog::Broken::Model(why))));
        }
    };
    let mut out = Vec::with_capacity(c.entries.len());
    for e in &c.entries {
        let q = NormalQuery::validated(
            c.schema(),
            (*e.terms).clone(),
            e.params.iter().flatten().copied().chain(e.unit).collect(),
            e.body,
        )
        .map_err(Refused::from);
        let q = match q {
            Ok(q) => q,
            Err(Refused::Untranslatable(u)) => return Ok(Err((e.name.clone(), u))),
            Err(Refused::Broken(n)) => return Err(Box::new(Broken::CallNotNormal(n))),
            Err(Refused::Model(why)) => {
                return Err(Box::new(Broken::Catalog(catalog::Broken::Model(why))));
            }
        };
        let (terms, params, body) = match p.query(&q) {
            Ok(q) => q.into_parts(),
            Err(Refused::Untranslatable(u)) => return Ok(Err((e.name.clone(), u))),
            Err(Refused::Broken(n)) => return Err(Box::new(Broken::TranslationNotNormal(n))),
            Err(Refused::Model(why)) => {
                return Err(Box::new(Broken::Catalog(catalog::Broken::Model(why))));
            }
        };
        let mut vars = params.into_iter();
        let mut next = || vars.next().expect("a translation keeps every parameter");
        let params = e.params.iter().map(|x| x.map(|_| next())).collect();
        let unit = e.unit.map(|_| next());
        out.push(Served {
            compiled: now.clone(),
            terms: Arc::new(terms),
            body,
            params,
            unit,
        });
    }
    Ok(Ok(out))
}

/// Returns the bundle file `text` checked at the version it pins, whatever its state.
pub(super) fn reread(over: &Over<'_>, text: &str) -> Result<Checked, Error> {
    let r = Registration::bundle(text)?;
    let h = r.header().expect("a bundle's registration has a header");
    match pinned(over, h, &mut Pins::default())? {
        Some(pin) => checked(&pin.over(), &r),
        None => checked(over, &r),
    }
}

fn joined<'a>(ms: impl Iterator<Item = &'a str>) -> String {
    ms.collect::<Vec<_>>().join("\n")
}

/// A registration of an open database, by which its callables are called. It does not keep the
/// database open: once the database closes, every call through it is refused.
#[derive(Debug)]
pub struct Registered<R: Runtime = adtb_sys::os::OsRuntime> {
    engine: Weak<Engine<R>>,
    checked: Arc<Checked>,
}

impl<R: Runtime> Clone for Registered<R> {
    fn clone(&self) -> Self {
        Registered {
            engine: self.engine.clone(),
            checked: self.checked.clone(),
        }
    }
}

impl<R: Runtime> Database<R> {
    /// Returns the registration `name` the database was opened with, binding to it and renewing
    /// its lease when it is a bundle.
    ///
    /// Fail with `Retired`, `BundleSuperseded` or `BundleExpired` for a bundle that is, and with
    /// `UnknownBundle` for a name no registration of this open has.
    pub fn registered(&self, name: &str) -> Result<Registered<R>, Error> {
        let e = &self.engine.enter()?;
        e.bind(name)?;
        let found = Registered::served(&self.engine, name);
        if found.is_none() {
            e.db.admit_database()
                .map_err(|err| e.write_error(super::TxnId(0), err))?;
        }
        found.ok_or_else(|| {
            Error::rejected(RejectReason::UnknownBundle {
                bundle: name.to_owned(),
            })
        })
    }
}

impl<R: Runtime> Registered<R> {
    pub fn name(&self) -> &str {
        &self.checked().name
    }

    /// Returns the version a bundle's header pins; `None` for a registration that is no bundle.
    pub fn pins(&self) -> Option<u64> {
        self.checked().header.as_ref().map(|h| h.pins)
    }

    /// Returns the ordinal of the query, transaction or imported function `name`, from 1 in
    /// declaration order.
    pub fn ordinal(&self, name: &str) -> Option<u32> {
        self.checked()
            .entries
            .iter()
            .position(|e| e.name == name)
            .map(|i| u32::try_from(i + 1).expect("a registration holds fewer than 2^32 calls"))
    }

    /// Call the query, transaction or imported function `ordinal` with `args`, in parameter
    /// order, as one transaction of its own.
    ///
    /// A query or function only reads and runs as a read-only transaction. A transaction is
    /// rerun on a serialization failure up to `opts.retry`, else its `retry n`, else the retry
    /// limit. The call passes admission as [`Database::query_text`] does.
    pub fn call(&self, ordinal: u32, args: &[Val], opts: &CallOptions) -> Result<Called, Error> {
        let e = self.engine()?;
        let e = &e.enter()?;
        let (entry, p, due) = self.admitted(e, ordinal, args, false, || {
            e.principal(opts.assert.as_ref())
        })?;
        let queued = self.queued(args, opts.priority, opts.deadline);
        let admitted = e.admit_program(p, &queued, Authority::Owner)?;
        let done = e.call(&admitted, opts, || {
            if due {
                e.renew(&self.checked.name);
            }
        })?;
        let value = self.checked.encoded(entry, one(e, entry, done.value))?;
        Ok(match entry.kind {
            Callable::Txn => Called::Committed(Committed {
                value,
                position: done.position,
                attempts: done.attempts,
            }),
            Callable::Query | Callable::Fn => Called::Rows(value),
        })
    }

    /// Call the query or function `ordinal` with `args` as [`call`](Registered::call) does.
    /// Returns its result as record batches of one schema, as the wire's Arrow format maps the
    /// result type.
    pub fn call_arrow(
        &self,
        ordinal: u32,
        args: &[Val],
        opts: &CallOptions,
    ) -> Result<ArrowStream, Error> {
        let rows = match self.call(ordinal, args, opts)? {
            Called::Rows(rows) => rows,
            Called::Committed(done) => done.value,
        };
        arrow::stream(self.checked.schema(), rows)
    }

    fn checked(&self) -> &Checked {
        &self.checked
    }

    /// Returns the engine, refusing a call once the database is closed.
    fn engine(&self) -> Result<Arc<Engine<R>>, Error> {
        self.engine.upgrade().ok_or_else(Error::closed)
    }

    /// Returns where a call with `args` queues, ranked `rank` in the bundle's priority class and
    /// waiting until `deadline`.
    pub(super) fn queued(
        &self,
        args: &[Val],
        rank: Option<u8>,
        deadline: Option<SystemTime>,
    ) -> Queued {
        let bytes = CALL_BYTES + args.iter().map(Val::size).sum::<usize>();
        Queued::new(self.priority(), rank, deadline, bytes)
    }

    fn priority(&self) -> Priority {
        self.checked
            .header
            .as_ref()
            .map_or(Priority::Regular, |h| h.priority)
    }

    fn admitted(
        &self,
        e: &Engine<R>,
        ordinal: u32,
        args: &[Val],
        read_only: bool,
        caller: impl FnOnce() -> Result<Val, Error>,
    ) -> Result<(&Entry, Prepared, bool), Error> {
        let checked = self.checked();
        let entry = checked
            .entry(ordinal)
            .ok_or_else(|| Error::rejected(RejectReason::UnknownQuery))?;
        let due = e.admit(&checked.name, &entry.name)?;
        let caller = caller()?;
        arguments(checked, entry, args)?;
        if read_only && entry.writes {
            return Err(Error::invalid("a read-only transaction does not write"));
        }
        let p = prepared(checked, entry, args, caller);
        Ok((entry, p, due))
    }
}

impl<R: Runtime> AdHocTransaction<'_, R> {
    /// Call the query, transaction or imported function `ordinal` of `reg` with `args` as one
    /// call of the transaction, for the transaction's principal. A registered transaction's
    /// block commits with the transaction.
    ///
    /// A call that fails applies nothing and leaves the transaction open, except on a
    /// serialization failure, which aborts the transaction.
    pub fn call(
        &mut self,
        reg: &Registered<R>,
        ordinal: u32,
        args: &[Val],
        opts: &TxnCallOptions,
    ) -> Result<InTxn, Error> {
        if !std::ptr::eq(self.engine, reg.engine.as_ptr()) {
            return Err(Error::invalid(
                "a registration calls only in a transaction of its own database",
            ));
        }
        let (entry, p, due) = reg.admitted(self.engine, ordinal, args, self.read_only, || {
            Ok(self.caller.clone())
        })?;
        let e = &self.engine.enter()?;
        let admitted = e.admit_program(
            p,
            &reg.queued(args, opts.priority, opts.deadline),
            Authority::Owner,
        )?;
        let rows = self.run(e, &admitted, opts, || {
            if due {
                e.renew(&reg.checked.name);
            }
        })?;
        let value = reg.checked.encoded(entry, one(e, entry, rows))?;
        Ok(match entry.kind {
            Callable::Txn => InTxn::Pending(value),
            Callable::Query | Callable::Fn => InTxn::Rows(value),
        })
    }
}

fn arguments(checked: &Checked, entry: &Entry, args: &[Val]) -> Result<(), Error> {
    if args.len() != entry.params.len() {
        return Err(Error::invalid(format!(
            "`{}` takes {} arguments, not {}",
            entry.name,
            entry.params.len(),
            args.len()
        )));
    }
    for (i, (ty, value)) in entry.types.iter().zip(args).enumerate() {
        if !super::value::valid(checked.schema(), ty, value) {
            return Err(Error::invalid(format!(
                "argument {} of `{}` is not a value of its declared parameter type",
                i + 1,
                entry.name
            )));
        }
    }
    Ok(())
}

/// Returns the kind of root the callables of a registration with `header` are.
fn root_kind(header: Option<&Header>) -> RootKind {
    match header {
        Some(h) if h.hides_new => RootKind::HidingQuery,
        _ => RootKind::Query,
    }
}

/// Returns the call of `entry` with `args` for `caller`, through its translation when it has
/// one, its arguments carried to the version the database is at.
fn prepared(checked: &Checked, entry: &Entry, args: &[Val], caller: Val) -> Prepared {
    let (compiled, terms, body, vars, unit) = match &entry.served {
        Some(s) => (&s.compiled, &s.terms, s.body, &s.params, s.unit),
        None => (
            &checked.compiled,
            &entry.terms,
            entry.body,
            &entry.params,
            entry.unit,
        ),
    };
    let map = entry
        .served
        .as_ref()
        .map(|s| pinned::Versions::new(checked.schema(), s.schema()));
    let carried = |ty: &Ty, v: &Val| match &map {
        Some(m) => pinned::forward(m, ty, v),
        None => v.clone(),
    };
    let mut params: Vec<(Var, Val)> = vars
        .iter()
        .zip(entry.types.iter().zip(args))
        .filter_map(|(x, (ty, v))| x.map(|x| (x, carried(ty, v))))
        .collect();
    params.extend(unit.map(|u| (u, Val::record(Vec::new()))));
    Prepared {
        compiled: compiled.clone(),
        terms: terms.clone(),
        items: Normal::new(vec![body], root_kind(checked.header.as_ref())),
        commands: Vec::new(),
        params,
        functions: BTreeSet::new(),
        caller,
        writes: entry.writes,
        retry: entry.retry,
        deferrable: false,
        issuer: None,
    }
}

/// Returns the one result of a call of `entry` among `rows`, the rows of its one item.
fn one<R: Runtime>(e: &Engine<R>, entry: &Entry, mut out: Vec<DynRows>) -> DynRows {
    out.pop().unwrap_or_else(|| {
        rows(
            e.schema(),
            entry.terms.ty(entry.body).clone(),
            vec![Val::record(Vec::new())],
        )
    })
}

/// One callable of a registration as a binding sees it, its types at the registration's version.
pub(super) struct Callee<'c> {
    pub(super) name: &'c str,
    pub(super) writes: bool,
    /// The id of the schema function an imported function calls.
    pub(super) function: Option<u64>,
    /// The `module::name` of that schema function.
    pub(super) function_name: Option<&'c str>,
    pub(super) names: &'c [String],
    pub(super) types: &'c [Ty],
    pub(super) result: &'c Ty,
}

impl Entry {
    fn callee(&self) -> Callee<'_> {
        Callee {
            name: &self.name,
            writes: self.writes,
            function: self.function,
            function_name: self.function_name.as_deref(),
            names: &self.names,
            types: &self.types,
            result: &self.result,
        }
    }
}

/// A call of a registration checked against its signature, for a task to run.
pub(super) struct Bound {
    /// Whether the call renews the bundle's lease once it starts.
    pub(super) due: bool,
    checked: Arc<Checked>,
    entry: usize,
}

impl<R: Runtime> Registered<R> {
    /// Returns the call of `ordinal` with `args` for `caller`, checked as
    /// [`call`](Registered::call) checks it.
    pub(super) fn bound(
        &self,
        e: &Engine<R>,
        ordinal: u32,
        args: &[Val],
        caller: Val,
    ) -> Result<(Bound, Prepared), Error> {
        let (_, prepared, due) = self.admitted(e, ordinal, args, false, || Ok(caller))?;
        let bound = Bound {
            due,
            entry: ordinal as usize - 1,
            checked: self.checked.clone(),
        };
        Ok((bound, prepared))
    }

    /// Returns the registration `name` the engine serves.
    pub(super) fn served(e: &Arc<Engine<R>>, name: &str) -> Option<Registered<R>> {
        e.registered
            .lock(&e.rt)
            .iter()
            .find(|c| c.name == name)
            .map(|c| Registered {
                engine: Arc::downgrade(e),
                checked: c.clone(),
            })
    }

    /// Returns the callable of `ordinal`.
    pub(super) fn callee(&self, ordinal: u32) -> Option<Callee<'_>> {
        Some(self.checked.entry(ordinal)?.callee())
    }

    /// Returns every callable of the registration with its ordinal, in ordinal order.
    pub(super) fn callees(&self) -> impl Iterator<Item = (u32, Callee<'_>)> {
        (1..).zip(self.checked.entries.iter().map(Entry::callee))
    }

    /// Returns the header of a bundle's registration.
    pub(super) fn header(&self) -> Option<&Header> {
        self.checked.header.as_ref()
    }

    /// Returns the schema of the version the registration is checked at.
    pub(super) fn schema(&self) -> &Arc<CommittedSchema> {
        self.checked.schema()
    }

    /// Refuse `args` unless they are the arguments of the callable of `ordinal`.
    pub(super) fn arguments(&self, ordinal: u32, args: &[Val]) -> Result<(), Error> {
        let entry = self
            .checked
            .entry(ordinal)
            .ok_or_else(|| Error::rejected(RejectReason::UnknownQuery))?;
        arguments(&self.checked, entry, args)
    }
}

impl Bound {
    pub(super) fn name(&self) -> &str {
        &self.checked.name
    }

    /// Returns the schema the registration is checked against.
    pub(super) fn schema(&self) -> &Arc<CommittedSchema> {
        self.checked.schema()
    }

    /// Returns the one result among `rows`, the rows of the call's one item, encoded at the
    /// version the registration is checked at.
    pub(super) fn result<R: Runtime>(
        &self,
        e: &Engine<R>,
        rows: Vec<DynRows>,
    ) -> Result<DynRows, Error> {
        let entry = &self.checked.entries[self.entry];
        self.checked.encoded(entry, one(e, entry, rows))
    }

    /// Returns `elems`, elements of type `ty` of the call's set result over `schema`, encoded at
    /// the version the registration is checked at.
    pub(super) fn elements(
        &self,
        schema: &Schema,
        ty: Ty,
        elems: Vec<Val>,
    ) -> Result<DynRows, Error> {
        let entry = &self.checked.entries[self.entry];
        self.checked.encoded(entry, rows(schema, ty, elems))
    }
}
