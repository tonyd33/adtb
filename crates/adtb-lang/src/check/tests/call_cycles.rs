//! Recursion through functions and aggregates: every cycle of one to three calls over functions
//! and the parts of aggregates is refused naming one function or aggregate on it, and the same
//! calls with the cycle's closing call left out check.

use super::*;

/// A function or an aggregate, by index, the aggregate calling from one of its parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    Function(usize),
    Aggregate(usize, Part),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Init,
    Step,
    Merge,
    Finish,
}

const PARTS: [Part; 4] = [Part::Init, Part::Step, Part::Merge, Part::Finish];

impl Node {
    fn name(self) -> String {
        match self {
            Node::Function(i) => format!("f{i}"),
            Node::Aggregate(i, _) => format!("a{i}"),
        }
    }

    fn same(self, other: Node) -> bool {
        self.name() == other.name()
    }

    /// Returns the refusal of a cycle this node is on.
    fn refusal(self) -> String {
        let kind = match self {
            Node::Function(_) => "function",
            Node::Aggregate(..) => "aggregate",
        };
        format!(
            "{kind} `{}` calls itself; recursion is written with `fix`",
            self.name()
        )
    }

    /// Returns an expression of type I64 over `arg` that calls this node.
    fn call(self, arg: &str) -> String {
        match self {
            Node::Function(_) => format!("{}({arg})", self.name()),
            Node::Aggregate(..) => format!(
                "(match {}(v for v in [{arg}]) | None -> 0 | Some o -> o.value)",
                self.name()
            ),
        }
    }

    /// Returns the declaration of this node, its body calling `callee` when given.
    fn decl(self, callee: Option<Node>) -> String {
        let call = |arg: &str| callee.map_or(arg.to_string(), |c| c.call(arg));
        match self {
            Node::Function(_) => format!("fn {} (x: I64) -> I64 = {}", self.name(), call("x")),
            Node::Aggregate(_, part) => {
                let at = |p: Part, plain: &str, arg: &str| {
                    if part == p && callee.is_some() {
                        call(arg)
                    } else {
                        plain.to_string()
                    }
                };
                format!(
                    "aggregate {} (x: I64) -> I64 = state {{ s: I64 }} init {{ s: {} }} \
                     step (st, x) -> {{ s: {} }} merge (l, r) -> {{ s: {} }} finish st -> {}",
                    self.name(),
                    at(Part::Init, "0", "0"),
                    at(Part::Step, "st.s + x", "x"),
                    at(Part::Merge, "l.s + r.s", "l.s"),
                    at(Part::Finish, "st.s", "st.s"),
                )
            }
        }
    }
}

/// Returns every sequence of one to three distinct nodes over three functions and two
/// aggregates, each aggregate calling from each of its parts.
fn cycles() -> Vec<Vec<Node>> {
    let mut nodes = vec![Node::Function(0), Node::Function(1), Node::Function(2)];
    for i in 0..2 {
        nodes.extend(PARTS.iter().map(|&p| Node::Aggregate(i, p)));
    }
    let mut out: Vec<Vec<Node>> = vec![Vec::new()];
    let mut all = Vec::new();
    for _ in 0..3 {
        let mut next = Vec::new();
        for seq in &out {
            for &n in &nodes {
                if seq.iter().all(|m: &Node| !m.same(n)) {
                    let mut s = seq.clone();
                    s.push(n);
                    next.push(s);
                }
            }
        }
        all.extend(next.iter().cloned());
        out = next;
    }
    all
}

/// Returns the schema text declaring each node of `seq`, each calling the next, the last
/// calling the first when `closed`.
fn text(seq: &[Node], closed: bool) -> String {
    seq.iter()
        .enumerate()
        .map(|(i, n)| {
            let next = seq.get(i + 1).copied();
            let callee = if closed { next.or(Some(seq[0])) } else { next };
            n.decl(callee)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_call_cycle_through_functions_and_aggregate_parts_is_refused_naming_a_member() {
    let mut failures = Vec::new();
    let seqs = cycles();
    let mut aggregates = 0;
    for seq in &seqs {
        aggregates += usize::from(seq.iter().any(|n| matches!(n, Node::Aggregate(..))));
        let closed = text(seq, true);
        let named: Vec<String> = seq.iter().map(|n| n.refusal()).collect();
        match program(&closed) {
            Err(ds) if ds.len() == 1 && named.contains(&ds[0]) => {}
            other => failures.push(format!(
                "{closed}\nwants one of {named:?}, got {:?}",
                other.map(drop)
            )),
        }
        let open = text(seq, false);
        if let Err(ds) = program(&open) {
            failures.push(format!("{open}\nwants it checked, got {ds:?}"));
        }
    }
    assert!(
        seqs.iter().any(|s| s.len() == 3),
        "the search reached three calls"
    );
    assert!(aggregates > 0, "the search reached an aggregate");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
