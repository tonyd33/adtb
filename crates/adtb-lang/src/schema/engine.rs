use super::{CtorRef, SchemaDefs, ValueDef};
use crate::types::Ty;

/// The constructors the engine names in the prelude's and its own declarations, and the
/// `Service` constructor of the schema's `Principal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineCtors {
    bool_false: CtorRef,
    bool_true: CtorRef,
    service: CtorRef,
    role: CtorRef,
    role_grant: CtorRef,
    deployment: CtorRef,
    violation: CtorRef,
}

impl EngineCtors {
    /// Preconditions: `defs` holds the prelude and the engine modules, its `Principal` declares
    /// `Service`, and `catalog::RoleGrant` declares its `GrantKind` as `kind`.
    pub(super) fn of(defs: &SchemaDefs) -> EngineCtors {
        let module = |name: &str| {
            defs.modules
                .iter()
                .find(|m| m.engine && m.name == name)
                .unwrap_or_else(|| panic!("the engine declares module `{name}`"))
        };
        let in_type = |m: &str, ty: &str, ctor: &str| {
            defs.types[module(m).types[ty].0 as usize]
                .ctor_named(ctor)
                .unwrap_or_else(|| panic!("`{m}::{ty}` declares `{ctor}`"))
        };
        let in_relation = |m: &str, rel: &str, ctor: &str| {
            let Some(ValueDef::Relation(r)) = module(m).values.get(rel) else {
                panic!("the engine declares relation `{m}::{rel}`")
            };
            let def = defs.relations[r.0 as usize].elem_def;
            defs.types[def.0 as usize]
                .ctor_named(ctor)
                .unwrap_or_else(|| panic!("`{m}::{rel}` declares `{ctor}`"))
        };
        let role_grant = in_relation("catalog", "grants", "RoleGrant");
        let deployment = in_type("catalog", "GrantKind", "Deployment");
        let grants = &defs.types[role_grant.def().0 as usize];
        let kind = grants
            .field(Some(role_grant), "kind")
            .map(|f| &grants.field_def(f).ty);
        assert!(
            matches!(kind, Some(Ty::Data { def, .. }) if *def == deployment.def()),
            "catalog RoleGrant declares its GrantKind as `kind`"
        );
        EngineCtors {
            bool_false: in_type("prelude", "Bool", "False"),
            bool_true: in_type("prelude", "Bool", "True"),
            service: defs.types[defs.principal.0 as usize]
                .ctor_named("Service")
                .expect("the schema's `Principal` declares `Service`"),
            role: in_relation("catalog", "roles", "Role"),
            role_grant,
            deployment,
            violation: in_relation("sys", "verify_findings", "Violation"),
        }
    }

    pub fn bool_false(&self) -> CtorRef {
        self.bool_false
    }

    pub fn bool_true(&self) -> CtorRef {
        self.bool_true
    }

    /// Returns `Service` of the schema's `Principal`.
    pub fn service(&self) -> CtorRef {
        self.service
    }

    /// Returns the row constructor of `catalog::roles`.
    pub fn role(&self) -> CtorRef {
        self.role
    }

    /// Returns the row constructor of `catalog::grants`.
    pub fn role_grant(&self) -> CtorRef {
        self.role_grant
    }

    /// Returns `catalog::GrantKind.Deployment`.
    pub fn deployment(&self) -> CtorRef {
        self.deployment
    }

    /// Returns the row constructor of `sys::verify_findings`.
    pub fn violation(&self) -> CtorRef {
        self.violation
    }
}
