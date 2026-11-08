//! Rename the fields of composite keys over generated relations and read back each stored row,
//! and each stored reference to it, with its values under the new names.

use std::ops::Range;

use adtb::Log;
use adtb_harness::{draw, seeds};
use adtb_sim::{Config, Sim};
use adtb_simkit::api;
use adtb_simkit::open;

const SEEDS: Range<u64> = 0..3;

/// The key field names, in sorted order.
const NAMES: [&str; 4] = ["c", "f", "i", "l"];

/// `GAPS[j]` sorts after `NAMES[j - 1]` and before `NAMES[j]`.
const GAPS: [&str; 5] = ["a", "d", "g", "j", "m"];

const TYPES: [&str; 3] = ["Text", "I64", "Bool"];

const ROWS: usize = 2;

/// Returns the value of key field `field`, of type `ty`, of row `row`: each differs from the
/// others of its row, and from the same field of the other rows, unless of type `Bool`.
fn literal(ty: &str, seed: u64, row: usize, field: usize) -> String {
    match ty {
        "Text" => format!("\"s{seed}r{row}f{field}\""),
        "I64" => format!("{}", seed * 100 + row as u64 * 10 + field as u64),
        _ => (row + field).is_multiple_of(2).to_string(),
    }
}

fn schema(names: &[&str], types: &[&str]) -> String {
    let fields: Vec<String> = names
        .iter()
        .zip(types)
        .map(|(n, t)| format!("{n}: {t}"))
        .collect();
    format!(
        "module shop\nadd module shop\nadd type Price = {{ {}, w: Text }}\n\
         add relation prices : Price key ({})\n\
         add type Order = {{ id: Key orders, price: Key prices }}\n\
         add relation orders : Order key id generate sequence\n",
        fields.join(", "),
        names.join(", "),
    )
}

/// Store `ROWS` rows of a relation keyed by fields `NAMES[..types.len()]` of types `types`, each
/// referenced by a row of `orders`, rename key field `NAMES[at]` to `to`, and read back each row
/// with its values under the new names, through the relation and through its reference.
fn renamed(seed: u64, types: &[&str], at: usize, to: &str) {
    let names = &NAMES[..types.len()];
    let first = schema(names, types);
    let rename = format!("module shop\nrename field Price.{} to {to}\n", names[at]);
    let after: Vec<&str> = (0..names.len())
        .map(|f| if f == at { to } else { names[f] })
        .collect();
    let case = format!(
        "seed {seed}: key {names:?} of {types:?}, `{}` to `{to}`",
        names[at]
    );
    let mut sim = Sim::new(seed, Config::default());
    api::run(&mut sim, |rt| {
        let db = open::opened(
            &rt,
            open::FILE,
            &Log::new([&first].iter().copied()),
            &open::options(2),
        );
        for row in 0..ROWS {
            let key: Vec<String> = (0..names.len())
                .map(|f| format!("{}: {}", names[f], literal(types[f], seed, row, f)))
                .collect();
            let key = key.join(", ");
            api::results(
                &db,
                &format!(
                    "transact\n  insert prices {{ {key}, w: \"w{row}\" }}\n  \
                     insert orders {{ price: prices.key({key}) }}"
                ),
            );
        }
    });
    api::run(&mut sim, |rt| {
        let db = open::opened(
            &rt,
            open::FILE,
            &Log::new([&first, &rename].iter().copied()),
            &open::options(2),
        );
        for row in 0..ROWS {
            let held = |x: &str| -> String {
                let terms: Vec<String> = (0..after.len())
                    .map(|f| format!("{x}.{} == {}", after[f], literal(types[f], seed, row, f)))
                    .collect();
                terms.join(" and ")
            };
            let want = vec![format!("\"w{row}\"")];
            let direct = format!("from x in prices where {} select x.w", held("x"));
            assert_eq!(want, api::query(&db, &direct), "{case}: row {row}");
            let followed = format!(
                "from o in orders where {} select o.price.w",
                held("o.price")
            );
            assert_eq!(
                want,
                api::query(&db, &followed),
                "{case}: row {row} by reference"
            );
        }
    });
}

#[test]
fn a_rename_of_a_composite_key_field_keeps_every_stored_row_and_reference() {
    let (mut reordered, mut kept, mut same_types) = (0, 0, 0);
    renamed(SEEDS.end, &["Text", "Text"], 0, GAPS[2]);
    same_types += 1;
    for seed in seeds::sweep(SEEDS) {
        let types: Vec<&str> = api::once(seed, |rt| {
            let len = 2 + draw::below(&rt, 3);
            (0..len)
                .map(|_| TYPES[draw::below(&rt, TYPES.len())])
                .collect()
        });
        renamed(seed, &types, 0, GAPS[0]);
        kept += 1;
        for at in 0..types.len() {
            for gap in (0..=types.len()).filter(|&j| j != at && j != at + 1) {
                renamed(seed, &types, at, GAPS[gap]);
                reordered += 1;
                let mut moved = types.clone();
                let ty = moved.remove(at);
                moved.insert(if gap < at { gap } else { gap - 1 }, ty);
                if moved == types {
                    same_types += 1;
                }
            }
        }
    }
    seeds::assert_reached("a rename that keeps the key's order", kept, 1);
    seeds::assert_reached("a rename that reorders the key", reordered, 1);
    seeds::assert_reached("a reorder that keeps the key's field types", same_types, 1);
}
