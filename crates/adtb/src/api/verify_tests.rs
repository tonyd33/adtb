use adtb_lang::ids::Stored;
use adtb_lang::ids::{CatCtorId, engine_ctor, engine_rel, held_id};
use adtb_lang::schema::{Schema, build};
use adtb_lang::types::RelId;

use super::{FindingRow, Findings, fields};
use crate::api::OpenError;
use crate::catalog::codec::testing::{Sample, Sampled, fields_reversed, reversed};
use crate::catalog::{KeptField, KeptRow, layout};
use crate::exec::Val;
use crate::key::Value;
use crate::storage::Pid;
use crate::write::Found;

fn table(schema: &Schema) -> RelId {
    crate::catalog::engine_relation(schema, engine_rel::VERIFY_FINDINGS)
}

fn kept(schema: &Schema, group: &str, ctor: Option<CatCtorId>, fields: Vec<KeptField>) -> KeptRow {
    KeptRow {
        relation: held_id(schema.ids().relation_id(table(schema))).raw(),
        group: group.into(),
        ctor,
        fields,
    }
}

fn row(schema: &Schema, ctor: Option<CatCtorId>, rest: Vec<KeptField>) -> KeptRow {
    let mut fields = vec![KeptField::Int(7), KeptField::Int(3), KeptField::Instant(11)];
    fields.extend(rest);
    kept(schema, "database", ctor, fields)
}

/// Field values by name.
type Named = Vec<(&'static str, Val)>;

/// Returns one finding of each way the writer records one, with the constructor it records and
/// the value of each field after the header, by name.
fn samples() -> Vec<(Found, CatCtorId, Named)> {
    use engine_ctor::{
        FINDING_CHECKSUM, FINDING_MISSING, FINDING_ORDER, FINDING_ORPHAN, FINDING_STALE_PAGE,
        FINDING_STRUCTURE, FINDING_VIOLATION,
    };
    let text = |s: &str| Val::text(s);
    vec![
        (
            Found::Checksum {
                at: "c".into(),
                page: None,
            },
            FINDING_CHECKSUM,
            vec![("where", text("c"))],
        ),
        (
            Found::StalePage {
                at: "s".into(),
                page: Pid(9),
            },
            FINDING_STALE_PAGE,
            vec![("where", text("s"))],
        ),
        (
            Found::Settled {
                at: "t".into(),
                page: Pid(9),
            },
            FINDING_STALE_PAGE,
            vec![("where", text("t"))],
        ),
        (
            Found::Order {
                index: 21,
                at: "o".into(),
            },
            FINDING_ORDER,
            vec![("index", Val::Int(21)), ("where", text("o"))],
        ),
        (
            Found::Orphan {
                index: 31,
                ctor: 32,
                at: "p".into(),
            },
            FINDING_ORPHAN,
            vec![
                ("index", Val::Int(31)),
                ("ctor", Val::Int(32)),
                ("where", text("p")),
            ],
        ),
        (
            Found::Missing {
                relation: 41,
                ctor: 42,
                index: 43,
                at: "m".into(),
            },
            FINDING_MISSING,
            vec![
                ("relation", Val::Int(41)),
                ("ctor", Val::Int(42)),
                ("index", Val::Int(43)),
                ("where", text("m")),
            ],
        ),
        (
            Found::Violation {
                constraint: 51,
                witness: "w".into(),
            },
            FINDING_VIOLATION,
            vec![("constraint", Val::Int(51)), ("witness", text("w"))],
        ),
        (
            Found::Structure { at: "x".into() },
            FINDING_STRUCTURE,
            vec![("where", text("x"))],
        ),
    ]
}

#[test]
fn a_kept_finding_reads_back_by_name_under_reversed_constructors_and_reversed_fields() {
    let shipped = build(&[]).expect("the engine's declarations build");
    let ctors = reversed();
    let fields_reversed = fields_reversed();
    let finding = shipped.ty(shipped.relation(table(&shipped)).elem_def);
    let n = u32::try_from(finding.ctors().len()).expect("a count fits 32 bits");
    for (f, expected, rest) in samples() {
        let written = fields(f.clone());
        let ctor = written.ctor;
        assert_eq!(expected, ctor, "{f:?}");
        let at = |schema: &Schema| {
            let c = schema
                .ids()
                .ctor_of(ctor)
                .expect("Finding declares the constructor");
            assert_eq!(schema.relation(table(schema)).elem_def, c.def(), "{ctor}");
            c.index()
        };
        assert_eq!(n - 1 - at(&shipped), at(&ctors), "{ctor}");
        let named: Vec<(&str, Val)> = [
            ("id", Val::Int(7)),
            ("run", Val::Int(3)),
            ("at", Val::Instant(11)),
        ]
        .into_iter()
        .chain(rest)
        .collect();
        for schema in [&shipped, &ctors, &fields_reversed] {
            let kept = kept(schema, "database", Some(ctor), written.fields((7, 3, 11)));
            let read = Findings::of(vec![kept], schema, table(schema)).expect("the row reads");
            let declared = layout::row(schema, table(schema), at(schema));
            assert_eq!(named.len(), declared.len(), "{ctor}");
            let fields = declared
                .iter()
                .map(|(d, _)| {
                    named
                        .iter()
                        .find(|(n, _)| n == d)
                        .unwrap_or_else(|| panic!("the sample names field `{d}`"))
                        .1
                        .clone()
                })
                .collect();
            let expected = Val::ctor(at(schema), fields);
            assert_eq!(
                vec![expected],
                read.vals().into_iter().collect::<Vec<_>>(),
                "{ctor}"
            );
        }
    }
    let read =
        Findings::of(Vec::new(), &fields_reversed, table(&fields_reversed)).expect("no row reads");
    let listed: Vec<usize> = (0..5).collect();
    let (_, declared) = read
        .codec
        .declared(engine_ctor::FINDING_ORDER, listed.clone())
        .expect("`Order` has five fields");
    assert_ne!(listed, declared);
}

#[test]
fn a_kept_row_of_another_shape_is_refused_as_damage_at_open() {
    use KeptField::{Instant, Int, Text};
    let schema = build(&[]).expect("the engine's declarations build");
    let page = || Text("page 9".into());
    let cases = [
        (
            row(&schema, Some(engine_ctor::FINDING_ORDER), vec![page()]),
            r"a kept row of group `database` and constructor 315 holds (7, 3, instant 11, `page 9`): a row of sys::verify_findings.Order holds 4 fields, not 5",
        ),
        (
            kept(
                &schema,
                "counters",
                Some(engine_ctor::FINDING_CHECKSUM),
                vec![Int(1), Int(2)],
            ),
            r"a kept row of group `counters` and constructor 313 holds (1, 2): a row of the counters of sys::verify_findings is not two integers of no constructor",
        ),
        (
            kept(
                &schema,
                "scrub cursor",
                Some(engine_ctor::FINDING_CHECKSUM),
                vec![Int(1), Int(2)],
            ),
            r"a kept row of group `scrub cursor` and constructor 313 holds (1, 2): a row of the scrub cursor of sys::verify_findings is not two integers of no constructor",
        ),
        (
            row(&schema, Some(engine_ctor::JOB_VERIFY), vec![page()]),
            r"a kept row of group `database` and constructor 297 holds (7, 3, instant 11, `page 9`): a row of sys::verify_findings names no constructor 297",
        ),
        (
            row(
                &schema,
                Some(Stored::<CatCtorId>::new(1_000_000).id()),
                vec![page()],
            ),
            r"a kept row of group `database` and constructor 1000000 holds (7, 3, instant 11, `page 9`): a row of sys::verify_findings names no constructor 1000000",
        ),
        (
            row(&schema, None, vec![page()]),
            r"a kept row of group `database` and no constructor holds (7, 3, instant 11, `page 9`): a row of sys::verify_findings names no constructor",
        ),
        (
            row(
                &schema,
                Some(engine_ctor::FINDING_CHECKSUM),
                vec![page(), page()],
            ),
            r"a kept row of group `database` and constructor 313 holds (7, 3, instant 11, `page 9`, `page 9`): a row of sys::verify_findings.Checksum holds 5 fields, not 4",
        ),
        (
            row(&schema, Some(engine_ctor::FINDING_CHECKSUM), vec![Int(9)]),
            r"a kept row of group `database` and constructor 313 holds (7, 3, instant 11, 9): a row of sys::verify_findings.Checksum holds 9 in field `where`",
        ),
        (
            row(
                &schema,
                Some(engine_ctor::FINDING_ORDER),
                vec![page(), page()],
            ),
            r"a kept row of group `database` and constructor 315 holds (7, 3, instant 11, `page 9`, `page 9`): a row of sys::verify_findings.Order holds `page 9` in field `index`",
        ),
        (
            kept(
                &schema,
                "database",
                Some(engine_ctor::FINDING_STRUCTURE),
                vec![Int(7), Instant(3), Instant(11), page()],
            ),
            r"a kept row of group `database` and constructor 473 holds (7, instant 3, instant 11, `page 9`): a row of sys::verify_findings.Structure holds instant 3 in field `run`",
        ),
        (
            row(
                &schema,
                Some(engine_ctor::FINDING_VIOLATION),
                vec![Int(-1), page()],
            ),
            r"a kept row of group `database` and constructor 318 holds (7, 3, instant 11, -1, `page 9`): a row of sys::verify_findings.Violation holds -1 in field `constraint`",
        ),
        (
            kept(&schema, "counters", None, vec![Int(1), Int(2), Int(3)]),
            r"a kept row of group `counters` and no constructor holds (1, 2, 3): a row of the counters of sys::verify_findings is not two integers of no constructor",
        ),
        (
            kept(&schema, "scrub cursor", None, vec![Int(1), page()]),
            r"a kept row of group `scrub cursor` and no constructor holds (1, `page 9`): a row of the scrub cursor of sys::verify_findings is not two integers of no constructor",
        ),
        (
            kept(&schema, "scrub cursor", None, vec![Int(-4), Int(2)]),
            r"a kept row of group `scrub cursor` and no constructor holds (-4, 2): a row of the scrub cursor of sys::verify_findings names page -4",
        ),
    ];
    for (row, text) in cases {
        let shown = format!("{row:?}");
        let Err(OpenError::Corrupt { detail }) = Findings::of(vec![row], &schema, table(&schema))
        else {
            panic!("{shown} is refused");
        };
        assert_eq!(text, detail, "{shown}");
    }
}

impl Sampled for FindingRow {
    fn samples(_: &Schema) -> Vec<Sample<FindingRow>> {
        use engine_ctor::{
            FINDING_CHECKSUM, FINDING_MISSING, FINDING_ORDER, FINDING_ORPHAN, FINDING_STALE_PAGE,
            FINDING_STRUCTURE, FINDING_VIOLATION,
        };
        let header = |id: i64| vec![("id", Value::I64(id)), ("run", Value::I64(id + 100))];
        let plain = |ctor, id: i64| {
            let want = FindingRow {
                id,
                run: id + 100,
                violated: None,
            };
            (ctor, header(id), want)
        };
        let mut violation = header(6);
        violation.push(("constraint", Value::I64(51)));
        vec![
            plain(FINDING_CHECKSUM, 1),
            plain(FINDING_STALE_PAGE, 2),
            plain(FINDING_ORDER, 3),
            plain(FINDING_ORPHAN, 4),
            plain(FINDING_MISSING, 5),
            (
                FINDING_VIOLATION,
                violation,
                FindingRow {
                    id: 6,
                    run: 106,
                    violated: Some(Stored::new(51).id()),
                },
            ),
            plain(FINDING_STRUCTURE, 7),
        ]
    }
}
