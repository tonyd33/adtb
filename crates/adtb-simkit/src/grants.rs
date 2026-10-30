use adtb::catalog::{Catalog, OWNER_ROLE, Rewrite, values};
use adtb::key::Value;
use adtb::mvcc::Transactions;
use adtb::runtime::Runtime;
use adtb::storage::Pager;
use adtb_lang::schema::Schema;
use adtb_lang::types::RelId;

/// Returns relation `catalog::name` of `schema` and its one constructor `ctor`.
pub fn catalog_relation(schema: &Schema, name: &str, ctor: &str) -> (RelId, u32) {
    let r = values::relation(schema, name);
    let c = schema
        .ty(schema.relation(r).elem_def)
        .ctor_index(ctor)
        .unwrap_or_else(|| panic!("catalog {name} declares {ctor}"));
    (r, c)
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
    let role = values::row(
        schema,
        r,
        c,
        vec![
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
    } = row;
    let schema = catalog.schema();
    let (r, c) = catalog_relation(schema, "grants", "RoleGrant");
    let no = values::boolean(schema, false);
    let target = match target {
        Some(t) => {
            let Value::Sum { ctor, .. } = values::tag(schema, "prelude", "Option", "Some") else {
                unreachable!("a tag is a sum");
            };
            Value::Sum {
                ctor,
                fields: vec![Value::Text(t.to_owned())],
            }
        }
        None => values::tag(schema, "prelude", "Option", "None"),
    };
    let grant = values::row(
        schema,
        r,
        c,
        vec![
            ("id", Value::I64(id)),
            (
                "kind",
                values::tag(schema, "catalog", "GrantKind", "Deployment"),
            ),
            ("privilege", Value::Text(privilege.to_owned())),
            ("target", target),
            ("grantee", Value::Text(grantee.to_owned())),
            ("grantor", Value::Text(grantor.to_owned())),
            ("grant_option", values::boolean(schema, grant_option)),
            ("dormant", no),
        ],
    );
    deploy(catalog, pager, txns, "grants", "RoleGrant", grant);
}
