use std::collections::BTreeMap;

use adtb_exec::exec;
use adtb_lang::capability::RootKind;
use adtb_lang::check::check_adhoc;
use adtb_lang::norm;
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_lang::permission;

use crate::plan::{self, fixture::Fixture};

const RAWS: &str = "type Raw = { id: Key raws, n: I64 }
relation raws : Raw key id generate sequence
  index (n)
type Cnt = { n: I64 }
relation cnts : Cnt key n
";

/// A chain of functions, each link calling the one before twice, so each link doubles the size of
/// the term an inlined call writes out.
#[derive(Clone, Copy, Debug)]
enum Chain {
    Sum,
    Branch,
    Union,
    Map,
    Read,
}

impl Chain {
    fn decls(self, links: usize) -> String {
        let (name, first, next) = match self {
            Chain::Sum => (
                "d",
                "(x: I64) -> I64 = x + 1",
                "(x: I64) -> I64 = {p}(x) + {p}(x)",
            ),
            Chain::Branch => (
                "u",
                "(x: I64) -> Set I64 = [x]",
                "(x: I64) -> Set I64 = if x > {i} then {p}(x) else {p}(x)",
            ),
            Chain::Union => (
                "s",
                "(x: I64) -> Set I64 = [x]",
                "(x: I64) -> Set I64 = {p}(x) union {p}(x)",
            ),
            Chain::Map => (
                "g",
                "(s: Set I64) -> Set I64 = from x in s where x < 5 select x + 1",
                "(s: Set I64) -> Set I64 = {p}(s) union {p}(s)",
            ),
            Chain::Read => (
                "h",
                "(x: I64) -> Set Cnt = from c in cnts where c.n == x select c",
                "(x: I64) -> Set Cnt = {p}(x) union {p}(x)",
            ),
        };
        let mut out = format!("{RAWS}fn {name}0 {first}\n");
        for i in 0..links {
            let link = next
                .replace("{p}", &format!("{name}{i}"))
                .replace("{i}", &i.to_string());
            out.push_str(&format!("fn {name}{} {link}\n", i + 1));
        }
        out
    }
}

/// A text over a chain of `links` links, ad hoc or a constraint.
#[derive(Clone, Copy, Debug)]
enum Witness {
    Adhoc(Chain, &'static str),
    Constraint(Chain, &'static str),
}

const WITNESSES: [Witness; 7] = [
    Witness::Adhoc(Chain::Sum, "from r in raws select d{n}(r.n)"),
    Witness::Adhoc(Chain::Branch, "from r in raws, y in u{n}(r.n) select y"),
    Witness::Adhoc(
        Chain::Branch,
        "from r in raws select count(y for y in u{n}(r.n))",
    ),
    Witness::Adhoc(Chain::Union, "from r in raws, y in s{n}(r.n) select y"),
    Witness::Adhoc(
        Chain::Map,
        "fix r = [1] union g{n}(r) in count(x for x in r)",
    ),
    Witness::Constraint(Chain::Read, "from r in raws, k in h{n}(r.n) select r"),
    Witness::Constraint(
        Chain::Branch,
        "from r in raws, y in u{n}(r.n) where y < 0 select r",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Pass {
    Check,
    Normalise,
    Permission,
    Plan,
    Code,
    Delta,
}

/// Returns what `f` returns and the allocations it made.
fn work<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = adtb_heap_cap::allocations();
    let out = f();
    (out, adtb_heap_cap::allocations() - before)
}

impl Witness {
    /// Returns the work of each pass over the text of `links` links.
    fn passes(self, links: usize) -> BTreeMap<Pass, usize> {
        let mut out = BTreeMap::new();
        match self {
            Witness::Adhoc(chain, text) => {
                let f = Fixture::new(&chain.decls(links));
                let text = text.replace("{n}", &links.to_string());
                let items = parse_adhoc(&text, Options::default())
                    .ok()
                    .unwrap_or_else(|ds| panic!("`{text}` does not parse: {ds:?}"));
                let (adhoc, w) = work(|| check_adhoc(f.compiled(), &items, &BTreeMap::new()));
                out.insert(Pass::Check, w);
                let adhoc = adhoc.unwrap_or_else(|ds| panic!("`{text}` does not check: {ds:?}"));
                let (normal, w) = work(|| norm::adhoc(f.compiled(), &adhoc));
                out.insert(Pass::Normalise, w);
                let normal =
                    normal.unwrap_or_else(|e| panic!("`{text}` has no normal form: {e:?}"));
                let [root] = normal.items() else {
                    panic!("`{text}` is one item")
                };
                let (required, w) =
                    work(|| permission::normal(f.schema(), &f.normal, normal.terms(), *root, &[]));
                out.insert(Pass::Permission, w);
                required.unwrap_or_else(|e| panic!("`{text}` has no requirements: {e:?}"));
                let (planned, w) = work(|| {
                    plan::plan(
                        f.schema(),
                        &f.normal,
                        &f.catalog,
                        normal.valid(),
                        *root,
                        RootKind::Adhoc,
                        plan::Options::default(),
                    )
                });
                out.insert(Pass::Plan, w);
                let planned = planned.unwrap_or_else(|e| panic!("`{text}` has no plan: {e:?}"));
                let (code, w) = work(|| exec::compile(f.schema(), &f.catalog, &planned, None));
                out.insert(Pass::Code, w);
                code.unwrap_or_else(|e| panic!("`{text}` does not compile: {e:?}"));
            }
            Witness::Constraint(chain, text) => {
                let text = text.replace("{n}", &links.to_string());
                let f = Fixture::new(&format!("{}constraint c = {text}\n", chain.decls(links)));
                let (refused, w) =
                    work(|| crate::write::refusals(f.schema(), &f.normal, &f.catalog));
                out.insert(Pass::Delta, w);
                refused.unwrap_or_else(|e| panic!("`{text}` breaks the checker: {e:?}"));
            }
        }
        out
    }
}

/// The fewest and most links of a witness the search measures.
const LINKS: std::ops::RangeInclusive<usize> = 1..=12;

#[test]
fn a_pass_over_a_chain_of_shared_calls_at_most_doubles_its_work_per_link() {
    let mut failed = Vec::new();
    for witness in WITNESSES {
        let mut before = witness.passes(*LINKS.start());
        for links in LINKS.skip(1) {
            let now = witness.passes(links);
            assert_eq!(
                before.keys().collect::<Vec<_>>(),
                now.keys().collect::<Vec<_>>(),
                "{witness:?}"
            );
            for (pass, w) in &now {
                if *w > 2 * before[pass] {
                    failed.push(format!(
                        "{witness:?} {pass:?}: {} at {} links, {w} at {links}",
                        before[pass],
                        links - 1
                    ));
                }
            }
            before = now;
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
