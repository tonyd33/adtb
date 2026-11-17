//! Write roles and grants into a catalog from a unit test.

use adtb_lang::ids::engine_ctor;
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::RelId;

use crate::catalog::codec::testing::{self, tag};
use crate::catalog::{Catalog, OWNER_ROLE, Rewrite, values};
use crate::key::Value;
use crate::mvcc::Transactions;
use crate::runtime::Runtime;
use crate::storage::Pager;

/// Returns relation `catalog::name` of `schema`.
pub fn relation(schema: &Schema, name: &str) -> RelId {
    let m = schema.module_named("catalog").expect("the catalog module");
    match schema.module(m).values.get(name) {
        Some(&ValueDef::Relation(r)) => r,
        _ => panic!("the catalog declares relation `{name}`"),
    }
}

/// Returns relation `catalog::name` of `schema` and its one constructor `ctor`.
pub fn catalog_relation(schema: &Schema, name: &str, ctor: &str) -> (RelId, u32) {
    let r = relation(schema, name);
    let c = schema
        .ty(schema.relation(r).elem_def)
        .ctor_index(ctor)
        .unwrap_or_else(|| panic!("catalog {name} declares {ctor}"));
    (r, c)
}

/// Insert the row of constructor `ctor` of `catalog::name` holding `fields`, durably.
pub fn deploy_fields<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    (name, ctor): (&str, &str),
    fields: Vec<(&str, Value)>,
) {
    let (r, c) = catalog_relation(catalog.schema(), name, ctor);
    let row = testing::row(catalog.schema(), r, c, &fields);
    deploy(catalog, pager, txns, name, ctor, row);
}

fn deploy<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    name: &str,
    ctor: &str,
    row: Vec<Value>,
) {
    let (relation, c) = catalog_relation(catalog.schema(), name, ctor);
    catalog
        .deploy(
            pager,
            txns,
            &[Rewrite {
                relation,
                deleted: Vec::new(),
                inserted: vec![(c, row)],
            }],
        )
        .expect("the deployment write commits");
}

/// Insert role `name` with id `id` into `catalog::roles`, durably.
pub fn create_role<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    id: i64,
    name: &str,
) {
    let schema = catalog.schema();
    let (r, c) = catalog_relation(schema, "roles", "Role");
    let role = testing::row(
        schema,
        r,
        c,
        &[
            ("id", Value::I64(id)),
            ("name", Value::Text(name.to_owned())),
            ("created", Value::Instant(0)),
        ],
    );
    deploy(catalog, pager, txns, "roles", "Role", role);
}

/// Insert a `Deployment` grant of capability `privilege` to `grantee` with id `id` into
/// `catalog::grants`, durably.
pub fn deployment_grant<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    id: i64,
    grantee: &str,
    privilege: &str,
) {
    deployment_row(
        catalog,
        pager,
        txns,
        id,
        DeploymentRow {
            grantee,
            grantor: OWNER_ROLE,
            privilege,
            target: None,
            grant_option: false,
            dormant: false,
        },
    );
}

/// What a `Deployment` row of `catalog::grants` holds besides its id.
#[derive(Clone, Copy, Debug)]
pub struct DeploymentRow<'a> {
    pub grantee: &'a str,
    pub grantor: &'a str,
    pub privilege: &'a str,
    pub target: Option<&'a str>,
    pub grant_option: bool,
    pub dormant: bool,
}

/// Insert `row` with id `id` into `catalog::grants`, durably.
pub fn deployment_row<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    id: i64,
    row: DeploymentRow<'_>,
) {
    let DeploymentRow {
        grantee,
        grantor,
        privilege,
        target,
        grant_option,
        dormant,
    } = row;
    let schema = catalog.schema();
    let (r, c) = catalog_relation(schema, "grants", "RoleGrant");
    let target = match target {
        Some(t) => values::some(Value::Text(t.to_owned())),
        None => values::none(),
    };
    let grant = testing::row(
        schema,
        r,
        c,
        &[
            ("id", Value::I64(id)),
            (
                "kind",
                tag(
                    schema,
                    (r, c),
                    "kind",
                    engine_ctor::GRANT_DEPLOYMENT,
                    Vec::new(),
                ),
            ),
            ("privilege", Value::Text(privilege.to_owned())),
            ("target", target),
            ("grantee", Value::Text(grantee.to_owned())),
            ("grantor", Value::Text(grantor.to_owned())),
            ("grant_option", values::boolean(grant_option)),
            ("dormant", values::boolean(dormant)),
        ],
    );
    deploy(catalog, pager, txns, "grants", "RoleGrant", grant);
}
