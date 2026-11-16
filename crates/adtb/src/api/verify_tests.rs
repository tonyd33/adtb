use adtb_lang::ids::{CatCtorId, engine_ctor, engine_rel, held_id};
use adtb_lang::schema::{Schema, build};
use adtb_lang::types::RelId;

use super::{Findings, fields};
use crate::api::OpenError;
use crate::catalog::{KeptField, KeptRow};
use crate::exec::Val;
use crate::storage::Pid;
use crate::write::Found;

fn engine() -> (Schema, RelId) {
    let schema = build(&[]).expect("the engine's declarations build");
    let table = crate::catalog::engine_relation(&schema, engine_rel::VERIFY_FINDINGS);
    (schema, table)
}

fn row(schema: &Schema, table: RelId, ctor: Option<CatCtorId>, rest: Vec<KeptField>) -> KeptRow {
    let mut fields = vec![KeptField::Int(7), KeptField::Int(3), KeptField::Instant(11)];
    fields.extend(rest);
    KeptRow {
        relation: held_id(schema.ids().relation_id(table)).raw(),
        group: "database".into(),
        ctor,
        fields,
    }
}

#[test]
fn a_kept_finding_holds_the_id_of_its_constructor_and_reads_back_at_its_position() {
    let (schema, table) = engine();
    let at = || "page 9".to_owned();
    let found = [
        (
            Found::Checksum {
                at: at(),
                page: None,
            },
            "Checksum",
        ),
        (
            Found::StalePage {
                at: at(),
                page: Pid(9),
            },
            "StalePage",
        ),
        (
            Found::Settled {
                at: at(),
                page: Pid(9),
            },
            "StalePage",
        ),
        (Found::Order { index: 1, at: at() }, "Order"),
        (
            Found::Orphan {
                index: 1,
                ctor: 2,
                at: at(),
            },
            "Orphan",
        ),
        (
            Found::Missing {
                relation: 1,
                ctor: 2,
                index: 3,
                at: at(),
            },
            "Missing",
        ),
        (
            Found::Violation {
                constraint: 1,
                witness: at(),
            },
            "Violation",
        ),
        (Found::Structure { at: at() }, "Structure"),
    ];
    let finding = schema.ty(schema.relation(table).elem_def);
    for (f, name) in found {
        let (ctor, rest) = fields(f);
        let named = finding
            .ctor_named(name)
            .expect("Finding declares the constructor");
        assert_eq!(schema.ids().ctor_id(named), Some(ctor), "{name}");
        let kept = row(&schema, table, Some(ctor), rest.clone());
        let read = Findings::of(vec![kept], &schema, table).expect("the row reads");
        let mut vals = vec![Val::Int(7), Val::Int(3), Val::Instant(11)];
        vals.extend(rest.iter().map(|f| match f {
            KeptField::Int(n) => Val::Int(*n),
            KeptField::Instant(n) => Val::Instant(*n),
            KeptField::Text(t) => Val::text(t),
        }));
        let expected = Val::ctor(named.index(), vals);
        assert_eq!(
            vec![expected],
            read.vals(&schema).into_iter().collect::<Vec<_>>(),
            "{name}"
        );
    }
}

#[test]
fn a_kept_finding_naming_no_constructor_of_finding_is_refused_as_damage() {
    let (schema, table) = engine();
    let rest = vec![KeptField::Text("page 9".into())];
    for (ctor, text) in [
        (
            Some(engine_ctor::JOB_VERIFY),
            r#"a finding holds KeptRow { relation: 339, group: "database", ctor: Some(CatCtorId(297)), fields: [Int(7), Int(3), Instant(11), Text("page 9")] }"#,
        ),
        (
            Some(CatCtorId::from_raw(1_000_000)),
            r#"a finding holds KeptRow { relation: 339, group: "database", ctor: Some(CatCtorId(1000000)), fields: [Int(7), Int(3), Instant(11), Text("page 9")] }"#,
        ),
        (
            None,
            r#"a finding holds KeptRow { relation: 339, group: "database", ctor: None, fields: [Int(7), Int(3), Instant(11), Text("page 9")] }"#,
        ),
    ] {
        let kept = row(&schema, table, ctor, rest.clone());
        let Err(OpenError::Corrupt { detail }) = Findings::of(vec![kept], &schema, table) else {
            panic!("a finding of {ctor:?} is refused");
        };
        assert_eq!(text, detail, "{ctor:?}");
    }
}
