use crate::api::entered::Enters;
use std::sync::Arc;

use adtb_lang::evolve::{Granted, Privilege, target_covers};
use adtb_lang::types::Ty;

use crate::catalog::codec::GrantKind;
use crate::exec::Val;
use crate::runtime::Runtime;

use super::super::bundle::{Callable, EDITION, Row, Stored};
use super::super::registered::{Callee, Registered};
use super::super::{Engine, Error, OWNER_ID, RejectReason};
use super::grant::Calls;
use super::tables::{CallOrigin, Grant, Tables, service};
use super::task::{Task, TaskOptions, submit};
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
pub(super) struct Target<'t> {
    bundle: &'t str,
    query: &'t str,
    function: Option<u64>,
}

impl<'t> Target<'t> {
    pub(super) fn new(bundle: &'t str, query: &'t str, function: Option<u64>) -> Target<'t> {
        Target {
            bundle,
            query,
            function,
        }
    }
}

/// Returns whether `grants` let `role` make the call `target`.
pub(super) fn holds_call<R: Runtime>(
    e: &Engine<R>,
    calls: &Calls<'_, R>,
    grants: &[Grant],
    role: &str,
    target: &Target<'_>,
) -> bool {
    let call = format!("{}::{}", target.bundle, target.query);
    grants
        .iter()
        .filter(|g| g.grantee == role && g.privilege == Privilege::Call && !g.dormant)
        .any(|g| {
            let row = g.target.as_deref();
            match target.function {
                Some(id) => {
                    g.kind == GrantKind::Log
                        && row.and_then(|t| Granted::read_target(e.schema(), Privilege::Call, t))
                            == Some(Granted::CallFn(id))
                }
                None => {
                    g.kind == GrantKind::Deployment
                        && target_covers(e.schema(), calls, Privilege::Call, row, Some(&call))
                }
            }
        })
}

/// Returns whether role id `by` calls bundle row `row` with no grant.
pub(super) fn owns(stored: &Stored, row: Option<&Row>, by: i64) -> bool {
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
        let caller = service(e.enter().schema(), &session.name);
        let held = self.record_refusal(session, &caller, &origin, held)?;
        let e = &e.call()?;
        let row = e.deployed.read(&e.rt).bundles.get(bundle).cloned();
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
            binding: Arc::new(binding(e, &registered, &held)),
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
            let t = Tables::new(e.schema(), id, None);
            self.checked(session, &t, e, tx)?;
            let stored = e.deployed.read(&e.rt);
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
                    .any(|c| c.kind != Callable::Fn && holds(&c.name, None));
            Ok(declared.then_some(held))
        })
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
            .read(&e.rt)
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
        let e = self.session.engine.upgrade().ok_or_else(Error::closed)?;
        let (admitted, work, call, _open) = self.admit_registered(&e, ordinal, &args, &opts)?;
        submit(&e, (admitted, work), Some(call), opts)
    }

    pub(super) fn session(&self) -> &RoleSession<R> {
        &self.session
    }

    pub(super) fn registered(&self) -> &Registered<R> {
        &self.registered
    }

    pub(super) fn origin(&self, callee: &Callee<'_>, ordinal: u32) -> CallOrigin {
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
}
