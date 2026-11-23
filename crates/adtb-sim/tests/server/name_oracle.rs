//! A role that cannot read the catalog gets the same answer and the same audit rows for a name it
//! holds no grant on as for an absent one, over drawn ad hoc texts.

use adtb::{Capability, Database, Error, Log, TaskOptions};
use adtb_sim::SimRuntime;
use adtb_simkit::api::{printed, result};
use adtb_simkit::open::options;
use adtb_testkit::Rng;
use adtb_testkit::fuzz::{SCHEMA, adhoc_text};
use adtb_testkit::process::once;

use crate::server_files::finish;

/// The names of [`SCHEMA`] a drawn text may hold that are no keyword: relations, fields,
/// constructors and functions.
const NAMES: [&str; 17] = [
    "items",
    "tags",
    "id",
    "name",
    "n",
    "status",
    "item",
    "size",
    "label",
    "at",
    "Draft",
    "Sent",
    "Paid",
    "Small",
    "Large",
    "twice",
    "as_status",
];

/// The capabilities a role may hold beside `adhoc` without reading the catalog.
const CAPABILITIES: [Capability; 7] = [
    Capability::Assert,
    Capability::Monitor,
    Capability::Cancel,
    Capability::Maintain,
    Capability::Configure,
    Capability::ReadFiles,
    Capability::WriteFiles,
];

const TEXTS: u64 = 48;

/// Returns [`SCHEMA`] as the first migration of a log.
fn migration() -> String {
    let mut out = String::new();
    for line in SCHEMA.lines() {
        if let Some(rest) = line.strip_prefix("module ") {
            out.push_str(&format!("module {rest}\nadd module {rest}\n"));
        } else if line.starts_with("fn ") {
            out.push_str(&format!("define {line}\n"));
        } else if !line.is_empty() {
            out.push_str(&format!("add {line}\n"));
        }
    }
    out
}

/// Returns `text` with each identifier `from` outside a string literal written `to`, and whether
/// one was.
fn swapped(text: &str, from: &str, to: &str) -> (String, bool) {
    let mut out = String::new();
    let mut found = false;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '"' {
            out.push(c);
            while let Some((_, d)) = chars.next() {
                out.push(d);
                if d == '\\' {
                    if let Some((_, e)) = chars.next() {
                        out.push(e);
                    }
                } else if d == '"' {
                    break;
                }
            }
        } else if c.is_ascii_alphabetic() || c == '_' {
            let mut end = i + c.len_utf8();
            while let Some(&(j, d)) = chars.peek() {
                if !(d.is_ascii_alphanumeric() || d == '_') {
                    break;
                }
                end = j + d.len_utf8();
                chars.next();
            }
            let word = &text[i..end];
            if word == from {
                out.push_str(to);
                found = true;
            } else {
                out.push_str(word);
            }
        } else {
            out.push(c);
        }
    }
    (out, found)
}

fn rows(db: &Database<SimRuntime>, ctor: &str, role: &str) -> usize {
    printed(result(
        db,
        &format!("from a in catalog::audit.{ctor} where a.role == \"{role}\" select a.seq"),
    ))
    .len()
}

/// Returns the answer of `text` sent by `role`, and the `Denied` and `Call` rows it added.
fn answer(rt: &SimRuntime, db: &Database<SimRuntime>, role: &str, text: &str) -> String {
    let before = (rows(db, "Denied", role), rows(db, "Call", role));
    let got: Result<Vec<String>, Error> = db
        .role_authority()
        .session(role)
        .and_then(|s| s.submit_text(text, &[], TaskOptions::default()))
        .and_then(|task| finish(rt, &task));
    let after = (rows(db, "Denied", role), rows(db, "Call", role));
    format!(
        "{got:?}; Denied +{}, Call +{}",
        after.0 - before.0,
        after.1 - before.1
    )
}

/// For a role holding `adhoc` alone, and with each capability that reads no catalog, a drawn text
/// answers and audits the same with one of the schema's names swapped for an absent one.
#[test]
fn an_ungranted_name_answers_as_an_absent_one() {
    let log = migration();
    once(51, |rt| {
        let db = Database::open_in(
            rt.clone(),
            "oracle.adt",
            &Log::new([log.as_str()]),
            &options(2),
        )
        .expect("the database opens");
        let authority = db.role_authority();
        let mut roles = vec![("bare".to_owned(), None)];
        roles.extend(CAPABILITIES.map(|c| (format!("{c:?}").to_lowercase(), Some(c))));
        for (role, capability) in &roles {
            authority.create_role(role).expect("the role");
            authority
                .grant_capability(role, Capability::Adhoc, false)
                .expect("adhoc");
            if let Some(c) = capability {
                authority
                    .grant_capability(role, *c, false)
                    .expect("the capability");
            }
        }
        let mut swaps = 0;
        let mut refused = 0;
        for seed in 0..TEXTS {
            let text = adhoc_text(seed);
            let mut rng = Rng::new(seed);
            let held: Vec<&str> = NAMES
                .into_iter()
                .filter(|n| swapped(&text, n, n).1)
                .collect();
            if held.is_empty() {
                continue;
            }
            let name = held[rng.below(held.len() as u64) as usize];
            let (absent, _) = swapped(&text, name, &format!("{name}_absent"));
            for (role, _) in &roles {
                let present = answer(&rt, &db, role, &text);
                assert_eq!(present, answer(&rt, &db, role, &absent), "{role}: {text}");
                refused += usize::from(present.starts_with("Err(Rejected { reason: Denied"));
                swaps += 1;
            }
        }
        assert!(swaps >= 8 * (TEXTS as usize) / 2, "{swaps} swaps ran");
        assert_eq!(swaps, refused, "every swapped text is refused");
    });
}
