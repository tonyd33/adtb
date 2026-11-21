use adtb_lang::ids::engine_ctor as k;

use crate::catalog::codec::{Codec, ROLE_CREATED, ROLE_ID, ROLE_NAME, RoleRow};
use crate::catalog::{Catalog, Rewrite};
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
    let codec = Codec::<RoleRow>::new(schema);
    let held = catalog.committed_roles(pager, txns).map_err(corrupt)?;
    if held.values().any(|name| name == OWNER_ROLE) {
        return Ok(());
    }
    if !held.is_empty() {
        return Err(corrupt(Corruption::NoSchemaOwner));
    }
    let row = codec.encode(
        k::ROLE,
        [
            (ROLE_ID, Value::I64(OWNER_ID)),
            (ROLE_NAME, Value::Text(OWNER_ROLE.to_owned())),
            (ROLE_CREATED, Value::Instant(micros(pager.wall()))),
        ],
    );
    catalog
        .deploy(
            pager,
            txns,
            &[Rewrite {
                relation: codec.relation(),
                deleted: Vec::new(),
                inserted: vec![row],
            }],
        )
        .map(drop)
        .map_err(corrupt)
}
