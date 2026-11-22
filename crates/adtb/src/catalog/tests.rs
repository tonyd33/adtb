use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::evolve::Source;
use adtb_lang::schema::Schema;

use crate::key::Value;

use super::Rot;
use super::{Broken, ElementTexts, Error, check_order_keys, declared, engine_schema, fold, layout};

const FIRST: &str = "module app
add module app
add type Shape { id: Key shapes }
  = Circle { r: I64 }
  | Square { s: I64 }
add relation shapes : Shape key id generate sequence
add set Round of shapes = Circle
add unique (r) in shapes.Circle
define fn area (x: Shape) -> I64 = match x
  | Circle c -> 3 * c.r * c.r
  | Square q -> q.s * q.s
define fn stamp (t: Text) -> Text = host \"stamp\" version \"1.2\"
";

fn migrated(from: &Source, text: &str) -> Source {
    let file = adtb_lang::parse::parse_migration(text, adtb_lang::parse::Options::default())
        .ok()
        .expect("the migration parses");
    from.migrate(&file, text)
        .unwrap_or_else(|r| panic!("the migration applies: {r:?}"))
}

#[test]
fn a_stored_migration_that_does_not_parse_is_not_accepted() {
    let texts = BTreeMap::from([(1, FIRST.to_string()), (2, "module app\nadd".to_string())]);
    let Err(Error::Unaccepted(version, why)) = fold(&texts, &BTreeMap::new()) else {
        panic!("the log folds")
    };
    assert_eq!(2, version);
    assert_eq!(vec!["expected what to add, found the end of the text"], why);
}

#[test]
fn a_stored_migration_that_does_not_apply_is_not_accepted() {
    let first = BTreeMap::from([(1, FIRST.to_string())]);
    fold(&first, &BTreeMap::new()).expect("the first migration folds");
    let texts = BTreeMap::from([(1, FIRST.to_string()), (2, FIRST.to_string())]);
    let Err(Error::Unaccepted(version, why)) = fold(&texts, &BTreeMap::new()) else {
        panic!("the log folds")
    };
    assert_eq!(2, version);
    assert_eq!(
        vec![
            "module `app` already exists",
            "module `app` already declares type `Shape`",
            "module `app` already declares relation `shapes`",
            "module `app` already declares set `shapes.Round`",
            "module `app` already declares fn `area`",
            "module `app` already declares fn `stamp`",
            "`app::shapes.unique(r) in Circle` is declared twice",
        ],
        why
    );
}

#[test]
fn a_log_folds_with_its_own_dormant_roles_after_its_texts_folded_with_others() {
    let texts = BTreeMap::from([
        (1, FIRST.to_string()),
        (2, "module app\ngrant read shapes to web\n".to_string()),
    ]);
    let dormancy = |dormant: &BTreeMap<u64, BTreeSet<String>>| -> Vec<bool> {
        let (source, _, _, _) = fold(&texts, dormant).expect("the log folds");
        source.grants.iter().map(|g| g.dormant).collect()
    };
    let web = BTreeMap::from([(2, BTreeSet::from(["web".to_string()]))]);
    assert_eq!(vec![false], dormancy(&BTreeMap::new()));
    assert_eq!(vec![true], dormancy(&web));
    assert_eq!(vec![false], dormancy(&BTreeMap::new()));
}

fn sum(ctor: u32, fields: Vec<Value>) -> Vec<Value> {
    vec![Value::Sum { ctor, fields }]
}

#[test]
fn stored_order_keys_order_every_declared_sum() {
    let source = migrated(&Source::default(), FIRST);
    let schema = source.schema().expect("the schema builds");
    let keys = layout::new_order_keys(&schema, &layout::OrderKeys::new());
    assert_eq!(Ok(()), check_order_keys(&schema, &keys));
    let mut missing = keys.clone();
    let (last, _) = missing.pop_last().expect("an order key");
    let got = check_order_keys(&schema, &missing);
    assert_eq!(Err(Rot::NoOrderKey(last).into()), got);
    let mut reversed = keys.clone();
    let ids: Vec<u64> = layout::declared_sums(&schema)
        .into_iter()
        .find(|cs| cs.len() > 1)
        .expect("a sum of two constructors");
    let (a, b) = (keys[&ids[0]].clone(), keys[&ids[1]].clone());
    reversed.insert(ids[0], b);
    reversed.insert(ids[1], a);
    let got = check_order_keys(&schema, &reversed);
    assert_eq!(Err(Rot::OrderKeyOrder(ids[1]).into()), got);
}

#[test]
fn a_fill_that_fails_internally_fails_the_step_as_internal() {
    let (sa, sb, r, mut data) = gaining_a_field();
    let rid = super::ids::rel_id(&sa, r);
    let cid = super::ctor_id(&sa, r, 0);
    let rb = super::ids::relation(&sb, rid).expect("ts after");
    let fid = super::change::fields(&sb, rb, 0)
        .iter()
        .find(|f| f.name == "n")
        .and_then(|f| f.id)
        .expect("n has an id");
    let mut moved = super::change::Moved::default();
    moved.fills.insert(
        (rid, cid, fid),
        Box::new(|_, _| Err(Broken::LastNotSum.into())),
    );
    assert_eq!(
        Err(Error::from(Broken::LastNotSum)),
        super::change::step(&sa, &sb, &mut data, moved)
    );
}

/// Returns the schemas of a relation `ts` of a one-field record before and after it gains a
/// field `n`, the relation in the first, and data holding one row of it.
fn gaining_a_field() -> (Schema, Schema, adtb_lang::types::RelId, super::change::Data) {
    let a = migrated(
        &Source::default(),
        "module m\nadd module m\nadd type T = { id: Key ts }\nadd relation ts : T key id\n",
    );
    let b = migrated(&a, "module m\nadd field T.n: I64 fill 0\n");
    let (sa, sb) = (
        a.schema().expect("it builds"),
        b.schema().expect("it builds"),
    );
    let (r, _) = sa
        .ids()
        .relation_ids()
        .find(|&(r, _)| sa.relation(r).name == "ts")
        .expect("ts");
    let rid = super::ids::rel_id(&sa, r);
    let cid = super::ctor_id(&sa, r, 0);
    let mut data = super::change::Data::new();
    data.insert(rid, [(cid, vec![vec![Value::I64(1)]])].into());
    (sa, sb, r, data)
}

#[test]
fn a_rekey_of_a_relation_the_schema_after_lacks_is_internal() {
    let (sa, _, r, data) = gaining_a_field();
    let maps = [adtb_lang::evolve::RowMap::Rekey { relation: r }];
    match super::moved::moved(&sa, &engine_schema(), &maps, &data, &Default::default()) {
        Err(e) => assert_eq!(
            Error::from(Broken::RekeyedGone(super::ids::rel_id(&sa, r))),
            e
        ),
        Ok(_) => panic!("the rekey maps"),
    }
}

#[test]
fn a_copy_from_a_row_without_the_field_is_internal() {
    let (sa, sb, r, data) = gaining_a_field();
    let maps = [adtb_lang::evolve::RowMap::Copy {
        def: sa.relation(r).elem_def,
        ctors: vec![0],
        field: "n".into(),
    }];
    let moved =
        super::moved::moved(&sa, &sb, &maps, &data, &Default::default()).expect("the copy maps");
    let fill = moved.fills.values().next().expect("the copy fills `n`");
    match fill(&[], &[]) {
        Err(super::change::Failed::Internal(why)) => {
            assert_eq!(Broken::NotCopied("n".into()), *why);
        }
        other => panic!("not an internal failure: {other:?}"),
    }
}

#[test]
fn an_index_path_naming_no_field_is_internal() {
    let (sa, _, r, _) = gaining_a_field();
    let path = [adtb_lang::kernel::PathStep::Field("missing".into())];
    let got = super::rows::path_steps(&sa, r, 0, &path);
    let want = crate::exec::Broken::PathRowField {
        path: path.to_vec(),
        field: "missing".into(),
    };
    assert_eq!(Err(want.into()), got.map(|_| ()));
}

#[test]
fn an_executor_damage_in_index_keying_stays_damage_and_its_internal_stays_internal() {
    let rot = crate::exec::Rot::SpillCut;
    let got = super::broken(rot.clone().into());
    assert_eq!(Error::Corrupt(Rot::Exec(rot)), got);
    let internal = || crate::exec::Error::from(crate::exec::Broken::EmptyRange);
    let want = Error::from(Broken::IndexKey(internal()));
    assert_eq!(want, super::broken(internal()));
}

#[test]
fn a_stored_row_without_the_shape_of_an_index_path_is_corrupt() {
    use super::rows::{Step, at_path};
    let steps = [Step::Field(0), Step::Ctor(1), Step::Field(0)];
    let held = [sum(1, vec![Value::I64(7)]).remove(0)];
    assert_eq!(Ok(Some(&Value::I64(7))), at_path(&held, &steps));
    let other = [sum(0, vec![]).remove(0)];
    assert_eq!(Ok(None), at_path(&other, &steps));
    for row in [vec![], vec![Value::I64(1)], vec![sum(1, vec![]).remove(0)]] {
        let got = at_path(&row, &steps);
        let rot = crate::exec::Rot::StoredPath(steps.to_vec());
        assert_eq!(Err(crate::exec::Error::Corrupt(rot)), got, "{row:?}");
    }
}

#[test]
fn a_derived_index_element_without_the_normal_program_is_internal() {
    let (sa, _, r, _) = gaining_a_field();
    let elements = [crate::plan::Element::Derived {
        index: 0,
        element: 0,
    }];
    let none = crate::exec::Broken::NoNormal {
        index: 0,
        element: 0,
    };
    let got = super::IndexKeys::new(&sa, None, r, &[0], &elements);
    assert_eq!(Some(none.clone().into()), got.err());
    let got = super::element_types(&sa, None, r, 0, &elements);
    assert_eq!(Err(none.into()), got);
}

/// Returns whether `line` reads a relation's key fields.
fn reads_key(line: &str) -> bool {
    [".key.", ".key;", ".key)", ".key_fields"]
        .iter()
        .any(|k| line.contains(k))
}

/// Returns whether `line` sorts: a sort method of any form, or an ordered collection built.
fn sorts(line: &str) -> bool {
    [
        ".sort(",
        ".sort_by",
        ".sort_unstable",
        ".sort_by_key",
        ".sort_by_cached_key",
    ]
    .iter()
    .any(|s| line.contains(s))
}

/// Add to `out` each line of the Rust sources under `dir` that orders a relation's key fields,
/// as `PATH: LINE TEXT`, and count in `seen` each sort it reads: a sort within 12 lines of a
/// read of the key, or a `BTreeSet` or `BTreeMap` collected in a statement that reads it.
fn key_sorts(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<String>,
    seen: &mut usize,
) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("the sources are readable")
        .map(|e| e.expect("a directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if !path.ends_with("target") {
                key_sorts(root, &path, out, seen);
            }
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") || path.ends_with("catalog/tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a source is readable");
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            let statement = || {
                let end = lines[i..].iter().position(|l| l.contains(';')).unwrap_or(0);
                &lines[i..=(i + end).min(i + 8)]
            };
            let hit = if sorts(l) {
                *seen += 1;
                lines[i.saturating_sub(12)..=i].iter().any(|l| reads_key(l))
            } else if l.contains("BTreeSet") || l.contains("BTreeMap") {
                *seen += 1;
                let st = statement();
                st.iter().any(|l| reads_key(l)) && st.iter().any(|l| l.contains("collect"))
            } else {
                false
            };
            if hit {
                let at = path.strip_prefix(root).expect("under the root").display();
                out.push(format!("{at}: {}", l.trim()));
            }
        }
    }
}

#[test]
fn only_key_names_sorts_the_fields_of_a_key() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (mut found, mut seen, mut scanned) = (Vec::new(), 0, Vec::new());
    let mut dirs: Vec<_> = std::fs::read_dir(root.join("crates"))
        .expect("the crates are readable")
        .map(|e| e.expect("a directory entry").path())
        .collect();
    dirs.sort();
    for dir in dirs {
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
        if manifest.contains("adtb-lang") {
            scanned.push(dir.file_name().expect("a crate directory").to_owned());
            key_sorts(&root, &dir, &mut found, &mut seen);
        }
    }
    for name in ["adtb", "adtb-lang"] {
        assert!(
            scanned.iter().any(|dir| dir == name),
            "the search includes {name}: {scanned:?}"
        );
    }
    assert!(
        seen > 100,
        "the search read the sorts of those crates: {seen}"
    );
    let membership = [
        "crates/adtb-lang/src/evolve/ctor/mod.rs: \
         pub(in crate::evolve) fn key_fields(schema: &Schema, def: TypeId) -> BTreeSet<String> {",
        "crates/adtb-lang/src/evolve/ids.rs: let keys: BTreeSet<&String> = schema",
    ];
    let rule = "crates/adtb-lang/src/textual/form.rs: names.sort();";
    assert_eq!(vec![membership[0], membership[1], rule], found);
}

#[test]
fn the_fields_of_one_constructor_are_written_in_name_order() {
    let source = migrated(
        &Source::default(),
        "module app\nadd module app\nadd type P = { k: I64, z: I64, a: I64 }\nadd relation ps : P key k\n",
    );
    let schema = source.schema().expect("it builds");
    let catalog = schema.module_named("catalog").expect("the catalog module");
    let Some(&adtb_lang::schema::ValueDef::Relation(fields)) =
        schema.module(catalog).values.get("fields")
    else {
        panic!("the catalog declares `fields`")
    };
    let app = schema.module_named("app").expect("the app module");
    let p = schema.ty(schema.module(app).types["P"]);
    let ctor = schema.ids().ctor_id(p.ctor_at(0).unwrap()).unwrap();
    let payload = schema
        .ty(schema.relation(fields).elem_def)
        .ctor_named("Payload")
        .unwrap()
        .index();
    let declared_fields = layout::row(&schema, fields, payload);
    let at = |name: &str| declared_fields.iter().position(|(n, _)| n == name).unwrap();
    let names: Vec<Value> = declared(
        &schema,
        &source,
        &ElementTexts::of(&source, &schema).unwrap(),
    )[&fields]
        .iter()
        .filter(|(c, row)| *c == payload && row[at("ctor")] == Value::I64(ctor.raw() as i64))
        .map(|(_, row)| row[at("name")].clone())
        .collect();
    assert_eq!(
        vec![
            Value::Text("a".to_string()),
            Value::Text("k".to_string()),
            Value::Text("z".to_string()),
        ],
        names
    );
}
