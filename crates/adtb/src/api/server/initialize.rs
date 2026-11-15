use crate::catalog::codec::{Codec, RoleRow};
use crate::catalog::{Catalog, Rewrite, layout};
use crate::key::Value;
use crate::mvcc::Transactions;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::super::cause::Corruption;
use super::super::{OWNER_ID, OWNER_ROLE, OpenError, corrupt, micros};

pub(in crate::api) fn initialize<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
) -> Result<(), OpenError> {
    let schema = catalog.schema();
    let roles = Codec::<RoleRow>::new(schema).relation();
    let ctor = schema.engine_ctors().role().index();
    let fields = layout::row(schema, roles, ctor);
    let held = catalog.committed_roles(pager, txns).map_err(corrupt)?;
    if held.values().any(|name| name == OWNER_ROLE) {
        return Ok(());
    }
    if !held.is_empty() {
        return Err(corrupt(Corruption::NoSchemaOwner));
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
                relation: roles,
                deleted: Vec::new(),
                inserted: vec![(ctor, row)],
            }],
        )
        .map_err(corrupt)
}
