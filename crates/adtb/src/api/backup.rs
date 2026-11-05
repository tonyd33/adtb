//! The commands `backup` and `restore`, and the test restore that proves a backup.

use adtb_lang::check::Command;
use adtb_lang::schema::Schema;
use adtb_lang::textual::print;
use adtb_lang::types::{Base, Ty};

use crate::catalog::{self, Dormant};
use crate::exec::Val;
use crate::runtime::Runtime;
use crate::storage::{self, BackupError, BackupRun, Envelope, Manifest, Proved};
use crate::write::{self, Bounds, Local, Stop, Txn};

use super::error::{FatalReason, RejectReason};
use super::text::{DynRows, Prepared, rows};
use super::{Database, Engine, Error, Log, OpenError, OpenOptions, Position, TxnId};

/// The catalog version of this engine's edition.
pub(super) const CATALOG_VERSION: u64 = 1;

/// Where a backup lives: a file system path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveTarget(String);

impl ArchiveTarget {
    /// Returns the target at `path`. Refuses a path with a URL scheme.
    pub fn path(path: impl AsRef<std::path::Path>) -> Result<ArchiveTarget, Error> {
        let text = path.as_ref().to_string_lossy().into_owned();
        refuse_scheme(&text)?;
        Ok(ArchiveTarget(text))
    }
}

/// The state a restore yields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestorePoint {
    /// The latest the source reaches.
    Latest,
    Position(Position),
    /// The state just before a schema version.
    Version(u64),
}

/// How far a backup is proven.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Proof {
    /// A test restore rebuilt the database and the full verification found nothing.
    Proven,
    /// The test restore found these, or the backup lists pages it copied damaged.
    Failed(Vec<String>),
    /// No test restore finished, or `function` is a host function verification needed and lacked.
    Unproven { function: Option<String> },
}

fn refuse_scheme(target: &str) -> Result<(), Error> {
    if let Some((scheme, _)) = target.split_once("://")
        && !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return Err(Error::invalid(format!(
            "`{target}`: a backup target is a file system path, not a `{scheme}` URL"
        )));
    }
    Ok(())
}

pub(super) fn refused(e: BackupError) -> Error {
    match e {
        BackupError::Occupied(target) => {
            Error::invalid(format!("the backup target `{target}` holds files"))
        }
        BackupError::Exists(path) => Error::invalid(format!("the restore path `{path}` exists")),
        BackupError::Absent(source) => Error::invalid(format!("no backup is at `{source}`")),
        BackupError::Damaged { part } => Error::rejected(RejectReason::BackupDamaged { part }),
        BackupError::UnknownFormat(format) => {
            Error::invalid(format!("this engine does not read backup format {format}"))
        }
        BackupError::OtherKind(kind) => {
            Error::invalid(format!("this engine does not restore a {kind}"))
        }
        BackupError::Stopped | BackupError::Quarantined => {
            Error::invalid("the backup stopped".to_owned())
        }
        BackupError::Io { what, error } => Error::storage(format!("`{what}`: {error:?}")),
    }
}

fn manifest_row(schema: &Schema, manifest: &str) -> DynRows {
    let ty = Ty::Record(vec![("manifest".into(), Ty::Base(Base::Text))]);
    let row = Val::record(vec![Val::text(manifest)]);
    rows(schema, ty, vec![row])
}

fn log_dir_of(options: &OpenOptions, path: &str) -> String {
    options.storage(path).log_dir()
}

/// What a restore wrote: the manifest of its source, the pages a backup lists as copied damaged,
/// and each grant of the restored file that is dormant.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct Restored {
    pub manifest: String,
    pub damaged: Vec<u64>,
    pub dormant: Vec<Dormant>,
}

fn restored<R: Runtime>(
    rt: &R,
    source: &str,
    path: &str,
    to: RestorePoint,
    options: &OpenOptions,
) -> Result<Restored, Error> {
    refuse_scheme(source)?;
    refuse_scheme(path)?;
    if storage::is_dump(rt, source).map_err(refused)? {
        let (manifest, dormant) = super::dump::restore_dump(rt, source, path, to, options)?;
        return Ok(Restored {
            manifest: manifest.text(),
            damaged: Vec::new(),
            dormant,
        });
    }
    let (manifest, _) = storage::check_backup(rt, source).map_err(refused)?;
    let end = Position::from(manifest.end);
    if to != RestorePoint::Latest && to != RestorePoint::Position(end) {
        return Err(unreachable_point(&manifest));
    }
    let manifest = restore_checked(rt, source, path, options)?;
    Ok(Restored {
        manifest: manifest.text(),
        damaged: manifest.damaged.iter().map(|pid| pid.0).collect(),
        dormant: Vec::new(),
    })
}

fn restore_checked<R: Runtime>(
    rt: &R,
    source: &str,
    path: &str,
    options: &OpenOptions,
) -> Result<Manifest, Error> {
    storage::restore_file(rt, source, path, &log_dir_of(options, path)).map_err(refused)
}

fn unreachable_point(manifest: &Manifest) -> Error {
    let end = Position::from(manifest.end);
    Error::rejected(RejectReason::RestorePointUnavailable {
        oldest: end,
        newest: end,
    })
}

/// Restore the backup at `archive` as the new database file `path` of runtime `rt`. Returns the
/// pages its manifest lists as copied damaged.
pub fn restore_in<R: Runtime>(
    rt: R,
    archive: &ArchiveTarget,
    path: &str,
    to: RestorePoint,
    options: &OpenOptions,
) -> Result<Vec<u64>, Error> {
    Ok(restored(&rt, &archive.0, path, to, options)?.damaged)
}

/// Restore `archive` as [`restore_in`] does. Returns what the restore wrote.
#[doc(hidden)]
pub fn restore_report_in<R: Runtime>(
    rt: R,
    archive: &ArchiveTarget,
    path: &str,
    to: RestorePoint,
    options: &OpenOptions,
) -> Result<Restored, Error> {
    restored(&rt, &archive.0, path, to, options)
}

/// Restore the backup at `archive` as the new database file at `path`. A relative `archive` and
/// `path` both resolve against the working directory. Returns the pages its manifest lists as
/// copied damaged.
#[cfg(target_os = "linux")]
pub fn restore(
    archive: &ArchiveTarget,
    path: impl AsRef<std::path::Path>,
    to: RestorePoint,
    options: OpenOptions,
) -> Result<Vec<u64>, Error> {
    let (rt, path) = host_runtime(path.as_ref(), &options)?;
    let source = std::path::absolute(&archive.0).map_err(|e| Error::invalid(e.to_string()))?;
    let archive = ArchiveTarget(source.to_string_lossy().into_owned());
    restore_in(rt, &archive, &path, to, &options)
}

/// Returns the host's runtime, and `path` as an absolute path it can open.
#[cfg(target_os = "linux")]
fn host_runtime(
    path: &std::path::Path,
    options: &OpenOptions,
) -> Result<(adtb_sys::os::OsRuntime, String), Error> {
    let absolute = std::path::absolute(path).map_err(|e| Error::invalid(e.to_string()))?;
    let dir = absolute.parent().unwrap_or(std::path::Path::new("/"));
    let rt = super::os_runtime(dir, options.workers());
    Ok((rt, absolute.to_string_lossy().into_owned()))
}

/// Rebuild the backup at `archive` in its scratch directory and run the full verification on the
/// result, recording what it found beside the manifest. A part that fails its check is an error.
///
/// `options` is what the database opens with for the test: its host functions above all.
pub fn prove_in<R: Runtime>(
    rt: R,
    archive: &ArchiveTarget,
    log: &Log,
    options: &OpenOptions,
) -> Result<Proof, Error> {
    let source = archive.0.as_str();
    storage::read_manifest(&rt, source).map_err(refused)?;
    let (manifest, text) = match storage::check_backup(&rt, source) {
        Err(e @ BackupError::Damaged { .. }) => {
            storage::withdraw_proof(&rt, source).map_err(refused)?;
            return Err(refused(e));
        }
        checked => checked.map_err(refused)?,
    };
    let hash = storage::manifest_hash(&text);
    let scratch = storage::make_scratch(&rt, source).map_err(refused)?;
    let file = storage::scratch_file(&scratch);
    if let Err(e) = restore_checked(&rt, source, &file, options) {
        storage::remove_scratch(&rt, &scratch).ok();
        return Err(e);
    }
    let proved = test_restore(&rt, &file, log, options, &manifest);
    storage::remove_scratch(&rt, &scratch).map_err(refused)?;
    storage::write_proof(&rt, source, &hash, &proved).map_err(refused)?;
    Ok(proof_of(proved))
}

/// Prove the backup at `archive` as [`prove_in`] does, under the host's runtime.
#[cfg(target_os = "linux")]
pub fn prove(archive: &ArchiveTarget, log: &Log, options: &OpenOptions) -> Result<Proof, Error> {
    let (rt, source) = host_runtime(std::path::Path::new(&archive.0), options)?;
    prove_in(rt, &ArchiveTarget(source), log, options)
}

/// Returns how far the proof file beside the manifest at `archive` proves it. A proof file torn,
/// absent or naming another manifest proves nothing.
pub fn proof_in<R: Runtime>(rt: &R, archive: &ArchiveTarget) -> Result<Proof, Error> {
    let (_, text) = storage::read_manifest(rt, &archive.0).map_err(refused)?;
    let hash = storage::manifest_hash(&text);
    let proved = storage::read_proof(rt, &archive.0, &hash).map_err(refused)?;
    Ok(proved.map_or(Proof::Unproven { function: None }, proof_of))
}

fn proof_of(proved: Proved) -> Proof {
    match proved {
        Proved::Proven => Proof::Proven,
        Proved::Failed(found) => Proof::Failed(found),
        Proved::Unproven { function } => Proof::Unproven {
            function: Some(function),
        },
    }
}

fn test_restore<R: Runtime>(
    rt: &R,
    file: &str,
    log: &Log,
    options: &OpenOptions,
    manifest: &Manifest,
) -> Proved {
    match opened_and_verified(rt, file, log, options, manifest) {
        Proved::Unproven { .. } if !manifest.damaged.is_empty() => {
            Proved::Failed(damage_lines(manifest))
        }
        proved => proved,
    }
}

fn damage_lines(manifest: &Manifest) -> Vec<String> {
    manifest
        .damaged
        .iter()
        .map(|pid| format!("page {} was copied damaged", pid.0))
        .collect()
}

fn opened_and_verified<R: Runtime>(
    rt: &R,
    file: &str,
    log: &Log,
    options: &OpenOptions,
    manifest: &Manifest,
) -> Proved {
    let opening = OpenOptions {
        create: None,
        ..options.clone()
    };
    let db = match Database::open_in(rt.clone(), file, log, &opening) {
        Ok(db) => db,
        Err(OpenError::HostFnMissing { name }) => return Proved::Unproven { function: name },
        Err(e) => return Proved::Failed(vec![e.to_string()]),
    };
    let verdict = verified(&db, manifest);
    match db.close() {
        Ok(()) => verdict,
        Err(e) => Proved::Failed(vec![e.to_string()]),
    }
}

fn verified<R: Runtime>(db: &Database<R>, manifest: &Manifest) -> Proved {
    let found = match db.verify_holding() {
        Ok(found) => found,
        Err(Error::Fatal {
            reason: FatalReason::MissingFunction { function },
            ..
        }) => return Proved::Unproven { function },
        Err(e) => return Proved::Failed(vec![e.to_string()]),
    };
    let form = found.form().clone();
    let mut lines = damage_lines(manifest);
    for row in found {
        lines.push(match row {
            Ok(v) => print(&v, &form).unwrap_or_else(|e| e.to_string()),
            Err(e) => e.to_string(),
        });
    }
    if lines.is_empty() {
        Proved::Proven
    } else {
        Proved::Failed(lines)
    }
}

/// Returns the directory a relative `backup` or `restore` path of a host database resolves
/// against: the file root, else the working directory.
#[cfg(target_os = "linux")]
pub(super) fn resolved_root(options: &OpenOptions) -> Option<std::path::PathBuf> {
    let root = options.file_root.clone().unwrap_or_else(|| ".".into());
    std::path::absolute(root).ok()
}

impl<R: Runtime> Engine<R> {
    /// Run `c` when it is `backup` or `restore`, whose refusals name their own reasons. Returns its
    /// rows, `None` for any other command.
    pub(super) fn file_command(
        &self,
        tx: &Txn,
        c: &Command,
        p: &Prepared,
        local: &mut Local,
        bounds: &Bounds,
    ) -> Option<Result<DynRows, Error>> {
        match c {
            Command::Backup { path, .. } => Some(self.backup_to(path, p, bounds)),
            Command::Dump {
                encrypt: Some(_), ..
            } => Some(Err(super::unsupported("`dump` with `encrypt`"))),
            Command::Dump { .. } if tx.wrote() => Some(Err(Error::invalid(
                "`dump` does not run in a transaction that has written",
            ))),
            Command::Dump { path, .. } => {
                let secured = p
                    .issuer
                    .as_ref()
                    .is_none_or(|i| i.holds.as_ref().is_some_and(|h| h.schema_capability));
                Some(
                    self.confine(path, p, bounds)
                        .and_then(|()| self.dump_to(tx, path, secured))
                        .map(|manifest| manifest_row(p.schema(), &manifest.text())),
                )
            }
            Command::Restore { from, to, point } => {
                let at = point.as_ref().map_or(Ok(RestorePoint::Latest), |point| {
                    self.point_of(tx, point, p, local, bounds)
                });
                Some(at.and_then(|at| self.restore_from(from, to, at, p, bounds)))
            }
            _ => None,
        }
    }

    /// Returns the point a `restore` names. An instant answers a version no backup reaches.
    fn point_of(
        &self,
        tx: &Txn,
        point: &adtb_lang::check::RestorePoint,
        p: &Prepared,
        local: &mut Local,
        bounds: &Bounds,
    ) -> Result<RestorePoint, Error> {
        use adtb_lang::check::RestorePoint as Named;
        let term = match point {
            Named::BeforeVersion(version) => return Ok(RestorePoint::Version(*version)),
            Named::AtInstant(_) => return Ok(RestorePoint::Version(0)),
            Named::AtPosition(term) => term,
        };
        let text = self.text_of(p, std::slice::from_ref(term));
        let failed = |e| self.write_error(TxnId(0), e);
        let mut got = self
            .db
            .statements(tx, text, local, bounds.clone())
            .map_err(|f| failed(f.error))?;
        let value = got
            .pop()
            .map(|outcome| outcome.result)
            .ok_or_else(|| failed(write::Broken::NoPointOutcome.into()))?;
        let at = write::position(&value).map_err(|e| failed(e.into()))?;
        Ok(at.map_or(RestorePoint::Version(0), |at| {
            RestorePoint::Position(Position::from(at))
        }))
    }

    pub(super) fn target(&self, path: &str) -> Result<String, Error> {
        refuse_scheme(path)?;
        let root = self.targets.lock(&self.rt).clone();
        Ok(match root {
            Some(root) => root.join(path).to_string_lossy().into_owned(),
            None => path.to_owned(),
        })
    }

    pub(super) fn envelope(&self) -> Envelope {
        let version = self
            .catalog
            .source()
            .log
            .last()
            .map_or(0, |migration| migration.number);
        let hash = catalog::version_hash(&self.schema, self.catalog.source());
        Envelope {
            release: env!("CARGO_PKG_VERSION").to_owned(),
            edition: super::bundle::EDITION.to_owned(),
            schema_version: version,
            schema_hash: hash.iter().map(|b| format!("{b:02x}")).collect(),
            catalog_version: CATALOG_VERSION,
        }
    }

    /// Refuse `path` of a call confined to the export roots unless its directory resolves inside
    /// one, with `Denied` recorded in the audit for a role.
    fn confine(&self, path: &str, p: &Prepared, bounds: &Bounds) -> Result<(), Error> {
        if !bounds.confined || self.db.files().exports_beside(path) {
            return Ok(());
        }
        let outside = format!("path `{path}` is outside export_roots");
        let e = match &p.issuer {
            Some(issuer) => self.refused_running(p, issuer, outside),
            None => write::Error::Denied(outside),
        };
        Err(self.write_error(TxnId(0), e))
    }

    fn backup_to(&self, path: &str, p: &Prepared, bounds: &Bounds) -> Result<DynRows, Error> {
        if let Some(refusal) = self.refusal() {
            return Err(refusal);
        }
        self.confine(path, p, bounds)?;
        let target = self.target(path)?;
        let stop = Stop::of(bounds, Some(self.rt.now()), None);
        let mut stopped = None;
        let mut visit = |_: storage::Pid, _: &mut [u8]| match stop.check(self.rt.now()) {
            Ok(()) => true,
            Err(e) => {
                stopped = Some(e);
                false
            }
        };
        let name = |q| self.db.quarantined_name(q);
        let mut run = BackupRun {
            target: &target,
            envelope: self.envelope(),
            visit: &mut visit,
            name: &name,
        };
        match self.pager.backup(&mut run) {
            Ok(manifest) => Ok(manifest_row(p.schema(), &manifest.text())),
            Err(BackupError::Stopped) => {
                let e = stopped.map_or(write::Error::Cancelled, write::Error::from);
                Err(self.write_error(TxnId(0), e))
            }
            Err(BackupError::Quarantined) => Err(self
                .refusal()
                .unwrap_or_else(|| refused(BackupError::Quarantined))),
            Err(e) => Err(refused(e)),
        }
    }

    fn restore_from(
        &self,
        from: &str,
        to: &str,
        at: RestorePoint,
        p: &Prepared,
        bounds: &Bounds,
    ) -> Result<DynRows, Error> {
        if bounds.confined {
            return Err(Error::invalid(
                "`restore` does not run through a server: it runs in process or on a closed file",
            ));
        }
        let (source, path) = (self.target(from)?, self.target(to)?);
        let options = OpenOptions::default();
        let restored = restored(&self.rt, &source, &path, at, &options)?;
        Ok(manifest_row(p.schema(), &restored.manifest))
    }
}
