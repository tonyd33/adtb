use std::collections::{BTreeMap, VecDeque};

use crate::history::{Dep, Edge};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Ww,
    Wr,
    Rw,
}

impl Kind {
    fn of(dep: &Dep) -> Kind {
        match dep {
            Dep::Ww { .. } => Kind::Ww,
            Dep::Wr { .. } => Kind::Wr,
            Dep::Rw { .. } => Kind::Rw,
        }
    }

    fn slot(self) -> usize {
        self as usize
    }
}

/// A dependency graph over nodes `0..n`, holding the first edge of each kind between two nodes.
#[derive(Debug)]
pub(crate) struct Graph {
    out: Vec<BTreeMap<usize, [Option<Edge>; 3]>>,
}

impl Graph {
    pub(crate) fn new(n: usize) -> Graph {
        Graph {
            out: vec![BTreeMap::new(); n],
        }
    }

    pub(crate) fn add(&mut self, from: usize, to: usize, edge: Edge) {
        assert_ne!(from, to, "self edge {edge:?}");
        let slot = &mut self.out[from].entry(to).or_default()[Kind::of(&edge.dep).slot()];
        slot.get_or_insert(edge);
    }

    fn step(&self, from: usize, to: usize, kinds: &[Kind]) -> Option<Edge> {
        let labels = self.out[from].get(&to)?;
        kinds.iter().find_map(|k| labels[k.slot()])
    }

    fn successors<'a>(
        &'a self,
        from: usize,
        kinds: &'a [Kind],
    ) -> impl Iterator<Item = usize> + 'a {
        self.out[from]
            .iter()
            .filter(|(_, labels)| kinds.iter().any(|k| labels[k.slot()].is_some()))
            .map(|(&to, _)| to)
    }

    /// Returns every edge of kind `kind` with both ends in `nodes`, with its ends.
    pub(crate) fn edges_within(&self, nodes: &[usize], kind: Kind) -> Vec<(usize, usize, Edge)> {
        let mut member = vec![false; self.out.len()];
        for &n in nodes {
            member[n] = true;
        }
        let mut found = Vec::new();
        for &from in nodes {
            for (&to, labels) in &self.out[from] {
                if let (true, Some(e)) = (member[to], labels[kind.slot()]) {
                    found.push((from, to, e));
                }
            }
        }
        found
    }

    /// Returns the strongly connected components of more than one node, over edges of `kinds`.
    pub(crate) fn components(&self, kinds: &[Kind]) -> Vec<Vec<usize>> {
        let n = self.out.len();
        let mut index = vec![usize::MAX; n];
        let mut low = vec![0; n];
        let mut on_stack = vec![false; n];
        let mut stack = Vec::new();
        let mut next = 0;
        let mut found = Vec::new();
        for root in 0..n {
            if index[root] != usize::MAX {
                continue;
            }
            let mut frames: Vec<(usize, Vec<usize>)> = Vec::new();
            index[root] = next;
            low[root] = next;
            next += 1;
            stack.push(root);
            on_stack[root] = true;
            frames.push((root, self.successors(root, kinds).collect()));
            while let Some((v, pending)) = frames.last_mut() {
                let v = *v;
                if let Some(w) = pending.pop() {
                    if index[w] == usize::MAX {
                        index[w] = next;
                        low[w] = next;
                        next += 1;
                        stack.push(w);
                        on_stack[w] = true;
                        frames.push((w, self.successors(w, kinds).collect()));
                    } else if on_stack[w] {
                        low[v] = low[v].min(index[w]);
                    }
                    continue;
                }
                frames.pop();
                if let Some((parent, _)) = frames.last() {
                    low[*parent] = low[*parent].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut component = Vec::new();
                    loop {
                        let w = stack.pop().expect("tarjan stack holds the component");
                        on_stack[w] = false;
                        component.push(w);
                        if w == v {
                            break;
                        }
                    }
                    if component.len() > 1 {
                        component.sort_unstable();
                        found.push(component);
                    }
                }
            }
        }
        found.sort();
        found
    }

    /// Returns a shortest path from `from` to `to` through `nodes` over edges of `kinds`.
    pub(crate) fn path(
        &self,
        from: usize,
        to: usize,
        nodes: &[usize],
        kinds: &[Kind],
    ) -> Option<Vec<Edge>> {
        let mut member = vec![false; self.out.len()];
        for &n in nodes {
            member[n] = true;
        }
        let mut parent = vec![usize::MAX; self.out.len()];
        parent[from] = from;
        let mut queue = VecDeque::from([from]);
        while let Some(v) = queue.pop_front() {
            if v == to {
                break;
            }
            for w in self.successors(v, kinds) {
                if member[w] && parent[w] == usize::MAX {
                    parent[w] = v;
                    queue.push_back(w);
                }
            }
        }
        if parent[to] == usize::MAX {
            return None;
        }
        let mut edges = Vec::new();
        let mut v = to;
        while v != from {
            let p = parent[v];
            edges.push(self.step(p, v, kinds).expect("BFS followed this edge"));
            v = p;
        }
        edges.reverse();
        Some(edges)
    }
}
