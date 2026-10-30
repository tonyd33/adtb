use adtb_lang::schema::ValueDef;

use crate::catalog::{Catalog, Rewrite, layout};
use crate::key::Value;
use crate::mvcc::Transactions;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::super::{OWNER_ID, OWNER_ROLE, OpenError, corrupt, micros};

pub(in crate::api) fn initialize<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
) -> Result<(), OpenError> {
    let schema = catalog.schema();
    let module = schema.module_named("catalog").expect("the catalog exists");
    let Some(ValueDef::Relation(roles)) = schema.module(module).values.get("roles") else {
        panic!("the catalog declares roles");
    };
    let ctor = schema
        .ty(schema.relation(*roles).elem_def)
        .ctor_index("Role")
        .expect("catalog roles declares Role");
    let fields = layout::row(schema, *roles, ctor);
    let name = fields
        .iter()
        .position(|(name, _)| name == "name")
        .expect("catalog roles declares name");
    let rows = catalog
        .committed_rows(pager, txns, *roles)
        .map_err(corrupt)?;
    if rows.iter().any(|(c, row)| {
        *c == ctor && matches!(row.get(name), Some(Value::Text(name)) if name == OWNER_ROLE)
    }) {
        return Ok(());
    }
    if !rows.is_empty() {
        return Err(corrupt("a nonempty roles relation has no schema owner"));
    }
    let row = fields
        .iter()
        .map(|(name, _)| match name.as_str() {
            "id" => Value::I64(OWNER_ID),
            "name" => Value::Text(OWNER_ROLE.to_owned()),
            "created" => Value::Instant(micros(pager.wall())),
            _ => panic!("catalog Role has only id, name and created"),
        })
        .collect();
    catalog
        .deploy(
            pager,
            txns,
            &[Rewrite {
                relation: *roles,
                deleted: Vec::new(),
                inserted: vec![(ctor, row)],
            }],
        )
        .map_err(corrupt)
}
