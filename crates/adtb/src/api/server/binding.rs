use std::sync::Arc;

use adtb_lang::evolve::{Granted, target_covers};
use adtb_lang::types::Ty;

use crate::exec::Val;
use crate::runtime::Runtime;

use super::super::bundle::{EDITION, Row, Stored};
use super::super::registered::{Callee, Registered};
use super::super::{Engine, Error, OWNER_ID, RejectReason};
use super::call::{RoleCall, Work};
use super::grant::Calls;
use super::session::Readmission;
use super::tables::{CallOrigin, CallRow, Grant, Tables, service};
use super::task::{TASK_BYTES, Task, TaskOptions, args_hash, submit};
use super::{RoleAuthority, RoleSession, denied};

/// What a handshake expects of the bundle it names. A field left `None` expects nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pins {
    /// The pinned schema version and its hash, 64 lowercase hex digits.
    pub version: Option<(u64, String)>,
    /// The catalog version the bundle pins.
    pub catalog: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    Query,
    Transaction,
    Function,
}

/// One callable of a bound bundle, its types at the pinned version.
#[derive(Clone, Debug)]
pub struct CallDescriptor {
    pub ordinal: u32,
    pub name: String,
    pub kind: CallKind,
    pub params: Vec<(String, Ty)>,
    pub result: Ty,
}

/// What a bind observed. A later call may still be refused.
#[derive(Clone, Debug)]
pub struct Binding {
    pub bundle: String,
    pub edition: String,
    /// The version a bundle pins, `None` for a registration that is no bundle.
    pub pins: Option<u64>,
    pub catalog: Option<u64>,
    /// When the bundle's lease runs out, in microseconds since the epoch.
    pub lease_until: Option<i64>,
    /// The callables the role may call when it binds. A later grant is callable by ordinal.
    pub calls: Vec<CallDescriptor>,
}

/// A role session bound to one registration. Every call rechecks lifecycle and authority.
#[derive(Clone, Debug)]
pub struct RoleBinding<R: Runtime = adtb_sys::os::OsRuntime> {
    session: RoleSession<R>,
    registered: Registered<R>,
    binding: Arc<Binding>,
}

/// A call of a bundle, by its ids at the bundle's pinned version.
struct Target<'t> {
    bundle: &'t str,
    query: &'t str,
    function: Option<u64>,
}

/// Returns whether `grants` let `role` make the call `target`.
fn holds_call<R: Runtime>(
    e: &Engine<R>,
    calls: &Calls<'_, R>,
    grants: &[Grant],
    role: &str,
    target: &Target<'_>,
) -> bool {
    let call = format!("{}::{}", target.bundle, target.query);
    grants
        .iter()
        .filter(|g| g.grantee == role && g.privilege == "call" && !g.dormant)
        .any(|g| {
            let row = g.target.as_deref();
            match target.function {
                Some(id) => {
                    g.kind == "Log"
                        && row.and_then(|t| Granted::read_target(&e.schema, "call", t))
                            == Some(Granted::CallFn(id))
                }
                None => {
                    g.kind == "Deployment"
                        && target_covers(&e.schema, calls, "call", row, Some(&call))
                }
            }
        })
}

/// Returns whether role id `by` calls bundle row `row` with no grant.
fn owns(stored: &Stored, row: Option<&Row>, by: i64) -> bool {
    by == OWNER_ID || row.is_some_and(|r| stored.owner_of(r) == by)
}

impl<R: Runtime> RoleAuthority<R> {
    /// Bind `session` to `bundle`, renewing its lease.
    ///
    /// Postconditions:
    /// - A role that may call nothing in the bundle is refused with `Denied`, recorded in the
    ///   audit, whether or not the bundle exists, and renews no lease. The schema owner and the
    ///   bundle's owner may call everything. `adhoc` alone does not bind.
    /// - The binding describes only the callables the role may call now, each by its ordinal.
    /// - Any other role fails with `UnknownBundle`, `VersionMismatch`, `CatalogMismatch`,
    ///   `Retired`, `BundleSuperseded` or `BundleExpired`, renewing no lease.
    pub fn bind(
        &self,
        session: &RoleSession<R>,
        bundle: &str,
        pins: &Pins,
    ) -> Result<RoleBinding<R>, Error> {
        let e = self.engine.upgrade().ok_or_else(Error::closed)?;
        let registered = Registered::served(&e, bundle);
        let held = self.callable(session, bundle, registered.as_ref());
        let held = held.and_then(|held| {
            held.ok_or_else(|| {
                denied(format!(
                    "role `{}` may call nothing in bundle {bundle}",
                    session.name
                ))
            })
        });
        let origin = CallOrigin::Registered {
            bundle: bundle.to_owned(),
            query: 0,
        };
        let caller = service(&e.schema, &session.name);
        let held = self.record_refusal(session, &caller, &origin, held)?;
        let _open = e.enter()?;
        let row = e.deployed.lock(&e.rt).bundles.get(bundle).cloned();
        let header = row
            .as_ref()
            .map(|r| &r.header)
            .or_else(|| registered.as_ref().and_then(Registered::header));
        if row.is_none() && registered.is_none() {
            return Err(unknown(bundle));
        }
        if let Some((version, hash)) = &pins.version
            && header.is_none_or(|h| h.pins != *version || h.hash != *hash)
        {
            return Err(Error::rejected(RejectReason::VersionMismatch {
                bundle: bundle.to_owned(),
                version: header.map_or(0, |h| i64::try_from(h.pins).unwrap_or(i64::MAX)),
            }));
        }
        if let Some(catalog) = pins.catalog
            && header.is_none_or(|h| h.catalog != Some(catalog))
        {
            let pinned = header.and_then(|h| h.catalog).unwrap_or(0);
            return Err(Error::rejected(RejectReason::CatalogMismatch {
                bundle: bundle.to_owned(),
                catalog: i64::try_from(pinned).unwrap_or(i64::MAX),
            }));
        }
        let due = e.admit(bundle, "")?;
        let registered = registered.ok_or_else(|| unknown(bundle))?;
        if due {
            e.renew(bundle);
        }
        Ok(RoleBinding {
            session: session.clone(),
            binding: Arc::new(binding(&e, &registered, &held)),
            registered,
        })
    }

    /// Returns the ordinals in `registered` the role of `session` may call, from one observation
    /// of its grants, or `None` when it may call nothing in `bundle`. Refuse a stale session with
    /// `Denied`.
    fn callable(
        &self,
        session: &RoleSession<R>,
        bundle: &str,
        registered: Option<&Registered<R>>,
    ) -> Result<Option<Vec<u32>>, Error> {
        self.access(false, |e, tx, id, _| {
            let t = Tables::new(&e.schema, id);
            self.checked(session, &t, e, tx)?;
            let stored = e.deployed.lock(&e.rt).clone();
            let row = stored.bundles.get(bundle);
            let callees = || registered.into_iter().flat_map(Registered::callees);
            if owns(&stored, row, session.id.0) {
                return Ok(Some(callees().map(|(ordinal, _)| ordinal).collect()));
            }
            let Some(row) = row else {
                return Ok(None);
            };
            let grants = t.grants(e, tx)?;
            let calls = Calls::new(e);
            let holds = |query: &str, function| {
                let target = Target {
                    bundle,
                    query,
                    function,
                };
                holds_call(e, &calls, &grants, &session.name, &target)
            };
            let held: Vec<u32> = callees()
                .filter(|(_, c)| holds(c.name, c.function))
                .map(|(ordinal, _)| ordinal)
                .collect();
            let declared = !held.is_empty()
                || stored
                    .calls_of(row.id)
                    .iter()
                    .any(|c| c.kind != "ImportedFn" && holds(&c.name, None));
            Ok(declared.then_some(held))
        })
    }

    /// Returns the principal of a call of `callee` of `registered`, from one observation of the
    /// role and its grants. Refuse with one `Denied` detail, recorded in the audit, a stale
    /// session, an assertion without `assert`, and a role holding no grant of the call.
    fn admit_call(
        &self,
        session: &RoleSession<R>,
        registered: &Registered<R>,
        callee: &Callee<'_>,
        origin: &CallOrigin,
        assertion: Option<&Val>,
    ) -> Result<Val, Error> {
        let bundle = registered.name();
        let observed = self.access(false, |e, tx, id, _| {
            let t = Tables::new(&e.schema, id);
            let principal = self.derive(session, &t, e, tx, assertion)?;
            let stored = e.deployed.lock(&e.rt).clone();
            if owns(&stored, stored.bundles.get(bundle), session.id.0) {
                return Ok((principal, true));
            }
            let target = Target {
                bundle,
                query: callee.name,
                function: callee.function,
            };
            let grants = t.grants(e, tx)?;
            let held = holds_call(e, &Calls::new(e), &grants, &session.name, &target);
            Ok((principal, held))
        });
        let caller = match observed {
            Ok((principal, true)) => return Ok(principal),
            Ok((principal, false)) => principal,
            Err(Error::Rejected {
                reason: RejectReason::Denied { .. },
            }) => {
                let e = self.engine.upgrade().ok_or_else(Error::closed)?;
                service(&e.schema, &session.name)
            }
            Err(err) => return Err(err),
        };
        let refused = denied(format!("role `{}` may not make this call", session.name));
        self.record_refusal(session, &caller, origin, Err(refused))
    }
}

fn unknown(bundle: &str) -> Error {
    Error::rejected(RejectReason::UnknownBundle {
        bundle: bundle.to_owned(),
    })
}

fn binding<R: Runtime>(e: &Engine<R>, registered: &Registered<R>, held: &[u32]) -> Binding {
    let s = *e.settings.lock(&e.rt);
    let name = registered.name();
    let header = registered.header();
    let calls = registered
        .callees()
        .filter(|(ordinal, _)| held.contains(ordinal))
        .map(|(ordinal, c)| CallDescriptor {
            ordinal,
            name: c.name.to_owned(),
            kind: match (c.function, c.writes) {
                (Some(_), _) => CallKind::Function,
                (None, true) => CallKind::Transaction,
                (None, false) => CallKind::Query,
            },
            params: c.names.iter().cloned().zip(c.types.to_vec()).collect(),
            result: c.result.clone(),
        })
        .collect();
    Binding {
        bundle: name.to_owned(),
        edition: header.map_or(EDITION, |h| &h.edition).to_owned(),
        pins: header.map(|h| h.pins),
        catalog: header.and_then(|h| h.catalog),
        lease_until: e
            .deployed
            .lock(&e.rt)
            .bundles
            .get(name)
            .map(|r| r.lease_until(&s)),
        calls,
    }
}

impl<R: Runtime> RoleBinding<R> {
    /// Returns what the bind observed.
    pub fn binding(&self) -> &Binding {
        &self.binding
    }

    /// Submit the call of `ordinal` with `args` as a task running for the session's role, or the
    /// principal `opts` asserts.
    ///
    /// The call runs with the rights of the bundle's owner. Authority and lifecycle are checked
    /// here and again when the task leaves the queue; a call that started finishes, retries
    /// included. A refusal for a lack of privilege writes a `Denied` row of `catalog::audit` and
    /// no `Call` row, and renews no lease. Every refused call has one detail, naming no callable.
    /// An ordinal past the bundle's callables fails with `UnknownQuery` before any of this. A call
    /// the queue has no room for writes no row.
    pub fn submit(
        &self,
        ordinal: u32,
        args: Vec<Val>,
        opts: TaskOptions,
    ) -> Result<Task<R>, Error> {
        let session = &self.session;
        let e = session.engine.upgrade().ok_or_else(Error::closed)?;
        let callee = self
            .registered
            .callee(ordinal)
            .ok_or_else(|| Error::rejected(RejectReason::UnknownQuery))?;
        let origin = self.origin(&callee, ordinal);
        let assertion = opts.call.assert.clone();
        let caller = session.authority().admit_call(
            session,
            &self.registered,
            &callee,
            &origin,
            assertion.as_ref(),
        )?;
        let _open = e.enter()?;
        self.registered.arguments(ordinal, &args)?;
        let typed: Vec<_> = callee
            .names
            .iter()
            .zip(callee.types.iter().zip(&args))
            .map(|(n, (t, v))| (n.as_str(), t, v))
            .collect();
        let hash = args_hash(self.registered.schema(), &typed)?;
        let bound = match self.registered.bound(&e, ordinal, &args, caller.clone()) {
            Ok(bound) => bound,
            Err(err) => {
                e.audit_call(&CallRow {
                    role: &session.name,
                    caller: &caller,
                    origin: &origin,
                    args: &hash,
                    failed: Some(&err),
                })?;
                return Err(err);
            }
        };
        let call = RoleCall {
            role: session.name.clone(),
            origin,
            args: hash,
            readmission: Some(Readmission::Registered {
                binding: self.clone(),
                ordinal,
                assertion,
            }),
        };
        let bytes = TASK_BYTES + args.iter().map(Val::size).sum::<usize>();
        let class = bound.priority;
        let work = Work::Registered(Box::new(bound));
        submit(&e, work, Some(call), opts, class, bytes)
    }

    fn origin(&self, callee: &Callee<'_>, ordinal: u32) -> CallOrigin {
        if let Some(name) = callee.function_name {
            return CallOrigin::Function {
                name: name.to_owned(),
            };
        }
        CallOrigin::Registered {
            bundle: self.registered.name().to_owned(),
            query: i32::try_from(ordinal).unwrap_or(i32::MAX),
        }
    }

    /// Refuse the call of `ordinal`, leaving the queue, as [`submit`](RoleBinding::submit) does.
    pub(super) fn recheck(
        &self,
        e: &Engine<R>,
        ordinal: u32,
        assertion: Option<&Val>,
    ) -> Result<(), Error> {
        let callee = self
            .registered
            .callee(ordinal)
            .expect("a submitted call's ordinal names a callable");
        let origin = self.origin(&callee, ordinal);
        self.session.authority().admit_call(
            &self.session,
            &self.registered,
            &callee,
            &origin,
            assertion,
        )?;
        e.admit(self.registered.name(), callee.name).map(drop)
    }
}
