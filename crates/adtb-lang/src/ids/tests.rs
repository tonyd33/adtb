use super::*;

/// Returns the objects of `slot`'s kind that the reverse lookup of `id` finds, `None` for a slot
/// with no reverse lookup.
fn found(ids: &CatalogIds, slot: &Slot, id: u64) -> Option<Vec<Slot>> {
    Some(match slot {
        Slot::Type(_) => ids.type_of(CatTypeId(id)).map(Slot::Type).collect(),
        Slot::Ctor(..) => ids
            .ctor_of(CatCtorId(id))
            .map(|(t, c)| Slot::Ctor(t, c))
            .collect(),
        Slot::Relation(_) => ids.relation_of(CatRelId(id)).map(Slot::Relation).collect(),
        Slot::Field(..) => ids
            .field_of(CatFieldId(id))
            .map(|(t, c, n)| Slot::Field(*t, *c, n.clone()))
            .collect(),
        Slot::Set(..) => ids
            .set_of(CatSetId(id))
            .map(|(r, n)| Slot::Set(r, n.to_string()))
            .collect(),
        Slot::Index(..) => ids
            .index_of(CatIndexId(id))
            .map(|(r, i)| Slot::Index(r, i))
            .collect(),
        Slot::Function(_) => ids
            .function_of(CatFunctionId(id))
            .map(Slot::Function)
            .collect(),
        Slot::Aggregate(_) => ids
            .aggregate_of(CatFunctionId(id))
            .map(Slot::Aggregate)
            .collect(),
        Slot::Constraint(_) => ids
            .constraint_of(CatConstraintId(id))
            .map(Slot::Constraint)
            .collect(),
        _ => return None,
    })
}

const REVERSED: [fn(u32) -> Slot; 9] = [
    |n| Slot::Type(TypeId(n)),
    |n| Slot::Ctor(TypeId(n), n),
    |n| Slot::Relation(RelId(n)),
    |n| Slot::Field(TypeId(n), Some(n), format!("f{n}")),
    |n| Slot::Set(RelId(n), format!("s{n}")),
    |n| Slot::Index(RelId(n), n as usize),
    |n| Slot::Function(FnId(n)),
    |n| Slot::Aggregate(n as usize),
    |n| Slot::Constraint(n as usize),
];

#[test]
fn a_reverse_lookup_finds_every_object_holding_an_id_least_first() {
    for (k, slot) in REVERSED.iter().enumerate() {
        let (a, b) = (slot(1), slot(2));
        let mut ids = CatalogIds::default();
        ids.set(&a, 1024);
        ids.set(&b, 1025);
        assert_eq!(
            Some(vec![a.clone()]),
            found(&ids, &a, 1024),
            "slot kind {k}"
        );
        assert_eq!(
            Some(vec![b.clone()]),
            found(&ids, &b, 1025),
            "slot kind {k}"
        );
        ids.set(&b, 1026);
        assert_eq!(Some(vec![]), found(&ids, &b, 1025), "slot kind {k}");
        assert_eq!(
            Some(vec![b.clone()]),
            found(&ids, &b, 1026),
            "slot kind {k}"
        );
        ids.set(&a, 1026);
        assert_eq!(Some(vec![]), found(&ids, &a, 1024), "slot kind {k}");
        let both = Some(vec![a.clone(), b.clone()]);
        assert_eq!(both, found(&ids, &a, 1026), "slot kind {k}");
        ids.forget(&a);
        assert_eq!(
            Some(vec![b.clone()]),
            found(&ids, &b, 1026),
            "slot kind {k}"
        );
    }
}
