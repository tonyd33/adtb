use adtb::catalog::{Catalog, OWNER_ROLE, Rewrite, layout};
use adtb::key::Value;
use adtb::mvcc::Transactions;
use adtb::runtime::Runtime;
use adtb::storage::Pager;
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::RelId;

/// Returns relation `catalog::name` of `schema` and its one constructor `ctor`.
pub fn catalog_relation(schema: &Schema, name: &str, ctor: &str) -> (RelId, u32) {
    let module = schema.module_named("catalog").expect("the catalog module");
    let Some(&ValueDef::Relation(r)) = schema.module(module).values.get(name) else {
        panic!("the catalog declares relation {name}");
    };
    let c = schema
        .ty(schema.relation(r).elem_def)
        .ctor_index(ctor)
        .unwrap_or_else(|| panic!("catalog {name} declares {ctor}"));
    (r, c)
}

/// Returns the value of the nullary constructor `ctor` of type `module::ty` of `schema`.
pub fn tag(schema: &Schema, module: &str, ty: &str, ctor: &str) -> Value {
    let m = schema.module_named(module).expect("an engine module");
    let def = schema.module(m).types[ty];
    Value::Sum {
        ctor: schema.ty(def).ctor_index(ctor).expect("a constructor"),
        fields: Vec::new(),
    }
}

/// Returns the stored row of `catalog::name` constructor `ctor` whose fields `value` gives by
/// name.
fn row(schema: &Schema, name: &str, ctor: &str, value: impl Fn(&str) -> Value) -> Vec<Value> {
    let (r, c) = catalog_relation(schema, name, ctor);
    layout::row(schema, r, c)
        .iter()
        .map(|(n, _)| value(n))
        .collect()
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
    let role = row(catalog.schema(), "roles", "Role", |n| match n {
        "id" => Value::I64(id),
        "name" => Value::Text(name.to_owned()),
        "created" => Value::Instant(0),
        other => panic!("catalog Role has no field {other}"),
    });
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
    let schema = catalog.schema();
    let grant = row(schema, "grants", "RoleGrant", |n| match n {
        "id" => Value::I64(id),
        "kind" => tag(schema, "catalog", "GrantKind", "Deployment"),
        "privilege" => Value::Text(privilege.to_owned()),
        "target" => tag(schema, "prelude", "Option", "None"),
        "grantee" => Value::Text(grantee.to_owned()),
        "grantor" => Value::Text(OWNER_ROLE.to_owned()),
        "grant_option" | "dormant" => tag(schema, "prelude", "Bool", "False"),
        other => panic!("catalog RoleGrant has no field {other}"),
    });
    deploy(catalog, pager, txns, "grants", "RoleGrant", grant);
}
