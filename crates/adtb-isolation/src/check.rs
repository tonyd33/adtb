use std::collections::{BTreeMap, BTreeSet};

use crate::graph::{Graph, Kind};
use crate::history::{Anomaly, Dep, Edge, Expected, Malformed, Op, Outcome, Txn, TxnId};

/// Returns every anomaly `history` shows, or why it cannot be analysed.
///
/// A transaction counts as committed when its outcome is `Committed`, or when it is `InDoubt` and
/// a committed transaction read one of its elements. Each key's version order is the longest
/// committed read of it. Every anomaly reported holds in every execution that produces these
/// observations.
///
/// Each cycle is one witness: one G0 cycle per strongly connected component of the write
/// dependencies; one G1c cycle per component of the write and read dependencies that contains
/// a read dependency; one G-single cycle, or failing that one G2 cycle, per component of all
/// dependencies that contains an anti-dependency.
pub fn check(history: &[Txn]) -> Result<Vec<Anomaly>, Malformed> {
    let h = Index::new(history)?;
    let mut anomalies = Vec::new();
    for &t in &h.members {
        internal(&history[t], &mut anomalies);
    }
    h.reads(&mut anomalies);
    let orders = h.orders(&mut anomalies);
    let graph = h.graph(&orders);
    cycles(&graph, &mut anomalies);
    Ok(anomalies)
}

struct Index<'h> {
    txns: &'h [Txn],
    writer: BTreeMap<(i64, i64), usize>,
    last_append: BTreeMap<(usize, i64), i64>,
    members: Vec<usize>,
    node: Vec<Option<usize>>,
}

impl<'h> Index<'h> {
    fn new(txns: &'h [Txn]) -> Result<Index<'h>, Malformed> {
        let mut ids = BTreeSet::new();
        let mut writer: BTreeMap<(i64, i64), usize> = BTreeMap::new();
        let mut last_append = BTreeMap::new();
        for (t, txn) in txns.iter().enumerate() {
            if !ids.insert(txn.id) {
                return Err(Malformed::DuplicateTxn(txn.id));
            }
            for op in &txn.ops {
                if let Op::Append { key, elem } = *op {
                    if let Some(&first) = writer.get(&(key, elem)) {
                        return Err(Malformed::DuplicateAppend {
                            key,
                            elem,
                            first: txns[first].id,
                            second: txn.id,
                        });
                    }
                    writer.insert((key, elem), t);
                    last_append.insert((t, key), elem);
                }
            }
        }
        let mut committed: Vec<bool> = txns
            .iter()
            .map(|t| t.outcome == Outcome::Committed)
            .collect();
        let mut work: Vec<usize> = (0..txns.len()).filter(|&t| committed[t]).collect();
        while let Some(t) = work.pop() {
            for (key, list) in reads(&txns[t]) {
                for &elem in list {
                    if let Some(&w) = writer.get(&(key, elem))
                        && !committed[w]
                        && txns[w].outcome == Outcome::InDoubt
                    {
                        committed[w] = true;
                        work.push(w);
                    }
                }
            }
        }
        let members: Vec<usize> = (0..txns.len()).filter(|&t| committed[t]).collect();
        let mut node = vec![None; txns.len()];
        for (n, &t) in members.iter().enumerate() {
            node[t] = Some(n);
        }
        Ok(Index {
            txns,
            writer,
            last_append,
            members,
            node,
        })
    }

    fn id(&self, t: usize) -> TxnId {
        self.txns[t].id
    }

    fn reads(&self, anomalies: &mut Vec<Anomaly>) {
        for &t in &self.members {
            let reader = self.id(t);
            for (key, list) in reads(&self.txns[t]) {
                let mut seen = BTreeSet::new();
                for &elem in list {
                    if !seen.insert(elem) {
                        anomalies.push(Anomaly::Duplicate { reader, key, elem });
                    }
                    match self.writer.get(&(key, elem)) {
                        None => anomalies.push(Anomaly::Garbage { reader, key, elem }),
                        Some(&w) if self.node[w].is_none() => anomalies.push(Anomaly::G1a {
                            reader,
                            writer: self.id(w),
                            key,
                            elem,
                        }),
                        Some(_) => {}
                    }
                }
                if let Some(&elem) = list.last()
                    && let Some(&w) = self.writer.get(&(key, elem))
                    && w != t
                    && self.last_append[&(w, key)] != elem
                {
                    anomalies.push(Anomaly::G1b {
                        reader,
                        writer: self.id(w),
                        key,
                        elem,
                    });
                }
            }
        }
    }

    /// Returns each key's version order, as far as the committed reads reveal it. A key whose
    /// reads disagree, or whose reads hold an element twice, has no order.
    fn orders(&self, anomalies: &mut Vec<Anomaly>) -> BTreeMap<i64, Vec<i64>> {
        let mut by_key: BTreeMap<i64, Vec<&[i64]>> = BTreeMap::new();
        for &t in &self.members {
            for (key, list) in reads(&self.txns[t]) {
                by_key.entry(key).or_default().push(list);
            }
        }
        let mut orders = BTreeMap::new();
        for (key, lists) in by_key {
            if lists.iter().any(|l| has_duplicate(l)) {
                continue;
            }
            let longest = *lists
                .iter()
                .max_by_key(|l| l.len())
                .expect("a key has a read");
            match lists.iter().find(|l| !longest.starts_with(l)) {
                Some(other) => anomalies.push(Anomaly::IncompatibleOrder {
                    key,
                    left: longest.to_vec(),
                    right: other.to_vec(),
                }),
                None => {
                    orders.insert(key, longest.to_vec());
                }
            }
        }
        orders
    }

    fn committed_writer(&self, key: i64, elem: i64) -> Option<usize> {
        self.writer
            .get(&(key, elem))
            .copied()
            .filter(|&w| self.node[w].is_some())
    }

    fn graph(&self, orders: &BTreeMap<i64, Vec<i64>>) -> Graph {
        let mut graph = Graph::new(self.members.len());
        let mut add = |from: usize, to: usize, key: i64, dep: Dep| {
            if from != to {
                let edge = Edge {
                    from: self.id(from),
                    to: self.id(to),
                    key,
                    dep,
                };
                graph.add(
                    self.node[from].expect("committed"),
                    self.node[to].expect("committed"),
                    edge,
                );
            }
        };
        let mut visible: BTreeMap<i64, Vec<(i64, usize)>> = BTreeMap::new();
        for (&key, order) in orders {
            let written: Vec<(i64, usize)> = order
                .iter()
                .filter_map(|&e| Some((e, self.committed_writer(key, e)?)))
                .collect();
            for pair in written.windows(2) {
                let ((prior, a), (next, b)) = (pair[0], pair[1]);
                add(a, b, key, Dep::Ww { prior, next });
            }
            visible.insert(key, written);
        }
        for &t in &self.members {
            for (key, list) in reads(&self.txns[t]) {
                let Some(written) = visible.get(&key) else {
                    continue;
                };
                let seen = list
                    .iter()
                    .filter(|&&e| self.committed_writer(key, e).is_some())
                    .count();
                let read = seen.checked_sub(1).map(|i| written[i]);
                if let Some((elem, w)) = read {
                    add(w, t, key, Dep::Wr { elem });
                }
                let installed = read.map(|(_, w)| w);
                let next = written[seen..].iter().find(|&&(_, w)| Some(w) != installed);
                if let Some(&(missed, w)) = next {
                    let read = read.map(|(e, _)| e);
                    add(t, w, key, Dep::Rw { read, missed });
                }
            }
        }
        graph
    }
}

fn reads(txn: &Txn) -> impl Iterator<Item = (i64, &[i64])> {
    txn.ops.iter().filter_map(|op| match op {
        Op::Read { key, list } => Some((*key, list.as_slice())),
        Op::Append { .. } => None,
    })
}

fn has_duplicate(list: &[i64]) -> bool {
    let mut seen = BTreeSet::new();
    !list.iter().all(|e| seen.insert(e))
}

fn internal(txn: &Txn, anomalies: &mut Vec<Anomaly>) {
    let mut state: BTreeMap<i64, Expected> = BTreeMap::new();
    for op in &txn.ops {
        match op {
            Op::Append { key, elem } => {
                let (Expected::List(l) | Expected::Suffix(l)) =
                    state.entry(*key).or_insert(Expected::Suffix(vec![]));
                l.push(*elem);
            }
            Op::Read { key, list } => {
                if let Some(expected) = state.get(key) {
                    let consistent = match expected {
                        Expected::List(l) => l == list,
                        Expected::Suffix(s) => list.ends_with(s),
                    };
                    if !consistent {
                        anomalies.push(Anomaly::Internal {
                            txn: txn.id,
                            key: *key,
                            expected: expected.clone(),
                            read: list.clone(),
                        });
                    }
                }
                state.insert(*key, Expected::List(list.clone()));
            }
        }
    }
}

const WRITES: &[Kind] = &[Kind::Ww];
const FLOWS: &[Kind] = &[Kind::Ww, Kind::Wr];
const ALL: &[Kind] = &[Kind::Ww, Kind::Wr, Kind::Rw];

fn cycles(graph: &Graph, anomalies: &mut Vec<Anomaly>) {
    let close = |nodes: &[usize], (from, to, edge): (usize, usize, Edge), kinds: &[Kind]| {
        let mut path = graph.path(to, from, nodes, kinds)?;
        path.insert(0, edge);
        Some(path)
    };
    for nodes in graph.components(WRITES) {
        let first = graph.edges_within(&nodes, Kind::Ww)[0];
        let cycle = close(&nodes, first, WRITES).expect("a component closes every edge in it");
        anomalies.push(Anomaly::G0 { cycle });
    }
    for nodes in graph.components(FLOWS) {
        if let Some(&first) = graph.edges_within(&nodes, Kind::Wr).first() {
            let cycle = close(&nodes, first, FLOWS).expect("a component closes every edge in it");
            anomalies.push(Anomaly::G1c { cycle });
        }
    }
    for nodes in graph.components(ALL) {
        let anti = graph.edges_within(&nodes, Kind::Rw);
        let Some(&first) = anti.first() else {
            continue;
        };
        let single = anti
            .iter()
            .filter_map(|&e| close(&nodes, e, FLOWS))
            .min_by_key(Vec::len);
        anomalies.push(match single {
            Some(cycle) => Anomaly::GSingle { cycle },
            None => Anomaly::G2 {
                cycle: close(&nodes, first, ALL).expect("a component closes every edge in it"),
            },
        });
    }
}
