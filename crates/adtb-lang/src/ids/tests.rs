use super::*;
use crate::schema::build;

/// Returns the object of `slot`'s kind that the reverse lookup of `id` finds, `None` for a slot
/// with no reverse lookup.
fn found(ids: &CatalogIds, slot: &Slot, id: u64) -> Option<Option<Slot>> {
    Some(match slot {
        Slot::Type(_) => ids.type_of(CatTypeId(id)).map(Slot::Type),
        Slot::Ctor(..) => ids.ctor_of(CatCtorId(id)).map(Slot::Ctor),
        Slot::Relation(_) => ids.relation_of(CatRelId(id)).map(Slot::Relation),
        Slot::Field(..) => ids.field_of(CatFieldId(id)).map(Slot::Field),
        Slot::Set(..) => ids
            .set_of(CatSetId(id))
            .map(|(r, n)| Slot::Set(r, n.to_string())),
        Slot::Index(..) => ids.index_of(CatIndexId(id)).map(|(r, i)| Slot::Index(r, i)),
        Slot::Function(_) => ids.function_of(CatFunctionId(id)).map(Slot::Function),
        Slot::Aggregate(_) => ids.aggregate_of(CatFunctionId(id)).map(Slot::Aggregate),
        Slot::Constraint(_) => ids.constraint_of(CatConstraintId(id)).map(Slot::Constraint),
        _ => return None,
    })
}

/// Returns field `n` of the first type of `s` with three fields.
fn field(s: &Schema, n: u32) -> FieldRef {
    let def = s
        .types
        .iter()
        .find(|t| t.field_refs().count() >= 3)
        .expect("a type with three fields");
    def.field_refs().nth(n as usize).expect("three fields").0
}

const REVERSED: [fn(&Schema, u32) -> Slot; 9] = [
    |_, n| Slot::Type(TypeId(n)),
    |s, n| Slot::Ctor(s.ty(s.prelude_type("Rounding")).ctor_at(n).unwrap()),
    |_, n| Slot::Relation(RelId(n)),
    |s, n| Slot::Field(field(s, n)),
    |_, n| Slot::Set(RelId(n), format!("s{n}")),
    |_, n| Slot::Index(RelId(n), n as usize),
    |_, n| Slot::Function(FnId(n)),
    |_, n| Slot::Aggregate(n as usize),
    |_, n| Slot::Constraint(n as usize),
];

#[test]
fn a_reverse_lookup_finds_the_object_holding_an_id() {
    let s = build(&[]).expect("the engine's schema builds");
    for (k, slot) in REVERSED.iter().enumerate() {
        let (a, b) = (slot(&s, 1), slot(&s, 2));
        let mut ids = CatalogIds::default();
        ids.set(&a, 1024);
        ids.set(&b, 1025);
        assert_eq!(
            Some(Some(a.clone())),
            found(&ids, &a, 1024),
            "slot kind {k}"
        );
        assert_eq!(
            Some(Some(b.clone())),
            found(&ids, &b, 1025),
            "slot kind {k}"
        );
        ids.set(&b, 1026);
        assert_eq!(Some(None), found(&ids, &b, 1025), "slot kind {k}");
        assert_eq!(
            Some(Some(b.clone())),
            found(&ids, &b, 1026),
            "slot kind {k}"
        );
        ids.forget(&b);
        assert_eq!(Some(None), found(&ids, &b, 1026), "slot kind {k}");
        ids.set(&a, 1026);
        assert_eq!(
            Some(Some(a.clone())),
            found(&ids, &a, 1026),
            "slot kind {k}"
        );
    }
}

#[test]
fn a_second_object_of_a_space_given_an_id_another_holds_panics() {
    let s = build(&[]).expect("the engine's schema builds");
    for (k, slot) in REVERSED.iter().enumerate() {
        let (a, b) = (slot(&s, 1), slot(&s, 2));
        let mut ids = CatalogIds::default();
        ids.set(&a, 1024);
        let shared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ids.set(&b, 1024)));
        let why = shared.expect_err("a shared id panics");
        assert_eq!(
            Some(&format!(
                "no two objects of a schema share a catalog id: {a:?} and {b:?} hold 1024"
            )),
            why.downcast_ref::<String>(),
            "slot kind {k}"
        );
    }
}

#[test]
fn objects_of_different_spaces_hold_one_id_each() {
    let s = build(&[]).expect("the engine's schema builds");
    let mut ids = CatalogIds::default();
    let slots: Vec<Slot> = REVERSED
        .iter()
        .map(|slot| slot(&s, 1))
        .filter(|x| !matches!(x, Slot::Ctor(_) | Slot::Relation(_) | Slot::Aggregate(_)))
        .collect();
    for x in &slots {
        ids.set(x, 1024);
    }
    for x in &slots {
        assert_eq!(Some(Some(x.clone())), found(&ids, x, 1024), "{x:?}");
    }
}
