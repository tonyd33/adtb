use std::sync::Weak;

use crate::exec::Val;
use crate::runtime::Runtime;

use super::super::{Engine, Error};
use super::tables::{RoleId, Tables};
use super::{Capability, RoleAuthority, require};

#[derive(Clone, Debug)]
pub struct RoleSession<R: Runtime = adtb_sys::os::OsRuntime> {
    pub(super) engine: Weak<Engine<R>>,
    pub(super) id: RoleId,
    pub(super) name: String,
}

impl<R: Runtime> RoleSession<R> {
    /// Returns the role name authenticated when the session was issued.
    pub fn role_name(&self) -> &str {
        &self.name
    }
}

impl<R: Runtime> RoleAuthority<R> {
    /// Derive a principal after checking the session and assertion capability.
    pub fn principal(
        &self,
        session: &RoleSession<R>,
        assertion: Option<&Val>,
    ) -> Result<Val, Error> {
        self.access(false, |e, tx, id, _| {
            let t = Tables::new(&e.schema, id);
            self.checked(session, &t, e, tx)?;
            let def = e.schema.ty(e.schema.principal);
            let service = def
                .ctor_index("Service")
                .expect("the Principal type declares Service");
            let Some(assertion) = assertion else {
                return Ok(Val::ctor(service, vec![Val::text(&session.name)]));
            };
            require(&t.grants(e, tx)?, session, Capability::Assert, false)?;
            e.principal(Some(assertion))
        })
    }
}
