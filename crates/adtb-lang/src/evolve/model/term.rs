use std::ops::Range;

use std::collections::BTreeMap;

use super::{Model, Obj};
use crate::canon::Names;
use crate::ids::{FIRST_LOG_ID, ObjectKey, Space};

/// Text written in the language, each name in it held as the objects it resolves to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Term(Vec<Piece>);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Text(String),
    /// A name resolving to `ids`, as `written`; `pun` when it also stands for a variable of
    /// that name.
    Name {
        ids: Vec<Obj>,
        written: String,
        pun: bool,
    },
    /// The name of a relation variable that operator `by` of the migration being applied adds.
    Ahead {
        by: usize,
        written: String,
    },
    /// A name whose objects an operator dropped, by their keys, until an operator adds an
    /// object of one of those keys.
    Dropped {
        keys: Vec<ObjectKey>,
        written: String,
        pun: bool,
    },
}

impl Piece {
    fn print(&self, model: &Model) -> String {
        match self {
            Piece::Text(s)
            | Piece::Ahead { written: s, .. }
            | Piece::Dropped { written: s, .. } => s.clone(),
            Piece::Name { ids, written, pun } => {
                let now = match ids.iter().find_map(|&o| model.name(o)) {
                    Some(now) => now,
                    None if ids.iter().all(|o| o.1 < FIRST_LOG_ID) => written,
                    None => panic!("a printed term names a gone object: {ids:?}"),
                };
                match (*pun, now != written) {
                    (true, true) => format!("{now}: {written}"),
                    (true, false) => written.clone(),
                    (false, _) => now.to_string(),
                }
            }
        }
    }
}

impl Term {
    /// Returns the text at `range` of `text`, each name `names` resolves there held by its ids,
    /// and each name `ahead` holds by the operator that adds its relation variable.
    pub(super) fn lift(
        text: &str,
        range: Range<usize>,
        names: &Names,
        ahead: &[(Range<usize>, usize)],
    ) -> Term {
        let mut found: Vec<(Range<usize>, Piece)> = names
            .ids_within(range.clone())
            .map(|(r, ids, pun)| {
                let piece = Piece::Name {
                    ids: ids.iter().copied().collect(),
                    written: text[r.clone()].to_string(),
                    pun,
                };
                (r, piece)
            })
            .chain(
                ahead
                    .iter()
                    .filter(|(r, _)| range.start <= r.start && r.end <= range.end)
                    .map(|(r, by)| {
                        let piece = Piece::Ahead {
                            by: *by,
                            written: text[r.clone()].to_string(),
                        };
                        (r.clone(), piece)
                    }),
            )
            .collect();
        found.sort_by_key(|(r, _)| (r.start, r.end));
        let mut pieces = Vec::new();
        let mut at = range.start;
        for (r, piece) in found {
            if r.start < at {
                continue;
            }
            if r.start > at {
                pieces.push(Piece::Text(text[at..r.start].to_string()));
            }
            pieces.push(piece);
            at = r.end;
        }
        if at < range.end {
            pieces.push(Piece::Text(text[at..range.end].to_string()));
        }
        Term(pieces)
    }

    /// Returns plain text holding no name.
    pub(super) fn text(s: &str) -> Term {
        Term(vec![Piece::Text(s.to_string())])
    }

    /// Returns a name resolving to `ids`, written `written`.
    pub(super) fn names(ids: Vec<Obj>, written: &str) -> Term {
        Term(vec![Piece::Name {
            ids,
            written: written.to_string(),
            pun: false,
        }])
    }

    /// Returns a name resolving to `obj`, written `written`.
    pub(super) fn name(obj: Obj, written: &str) -> Term {
        Term(vec![Piece::Name {
            ids: vec![obj],
            written: written.to_string(),
            pun: false,
        }])
    }

    /// Returns the term as `model` spells it: each name as its object is named now, a pun of a
    /// renamed field as `new: written`, and a name of dropped objects as written.
    pub(super) fn print(&self, model: &Model) -> String {
        self.0.iter().map(|p| p.print(model)).collect()
    }

    /// Insert `s` at byte `at` of the term as `model` prints it.
    ///
    /// Fail when `at` falls inside a name.
    pub(super) fn splice(&mut self, model: &Model, at: usize, s: &str) -> Result<(), String> {
        self.replace(model, at..at, Term::text(s))
    }

    /// Replace bytes `range` of the term as `model` prints it by `with`.
    ///
    /// Fail when an end of `range` falls inside a name.
    pub(super) fn replace(
        &mut self,
        model: &Model,
        range: Range<usize>,
        with: Term,
    ) -> Result<(), String> {
        let mut out = Vec::new();
        let mut with = Some(with.0);
        let mut start = 0;
        for p in std::mem::take(&mut self.0) {
            let text = p.print(model);
            let end = start + text.len();
            if end <= range.start {
                out.push(p);
            } else if start >= range.end {
                out.extend(with.take().into_iter().flatten());
                out.push(p);
            } else {
                let head = range.start > start;
                let tail = range.end < end;
                if (head || tail) && !matches!(p, Piece::Text(_)) {
                    return Err("a replacement inside a name".into());
                }
                if head {
                    out.push(Piece::Text(text[..range.start - start].to_string()));
                }
                out.extend(with.take().into_iter().flatten());
                if tail {
                    out.push(Piece::Text(text[range.end - start..].to_string()));
                }
            }
            start = end;
        }
        if let Some(rest) = with {
            if start < range.start {
                return Err("a replacement past the end of a term".into());
            }
            out.extend(rest);
        }
        self.0 = out;
        Ok(())
    }

    /// Returns a term of the pieces of each of `terms` in order.
    pub(super) fn concat(terms: impl IntoIterator<Item = Term>) -> Term {
        Term(terms.into_iter().flat_map(|t| t.0).collect())
    }

    /// Returns whether the term holds a name of dropped objects.
    pub(super) fn dropped(&self) -> bool {
        self.0.iter().any(|p| matches!(p, Piece::Dropped { .. }))
    }

    /// Returns the objects the term names.
    pub(super) fn objects(&self) -> impl Iterator<Item = Obj> + '_ {
        self.0
            .iter()
            .flat_map(|p| match p {
                Piece::Name { ids, .. } => ids.as_slice(),
                Piece::Text(_) | Piece::Ahead { .. } | Piece::Dropped { .. } => &[],
            })
            .copied()
    }

    /// Hold each name of a relation variable operator `by` adds by `relation`, its id.
    pub(super) fn added(&mut self, by: usize, relation: u64) {
        for p in &mut self.0 {
            if let Piece::Ahead { by: k, written } = p
                && *k == by
            {
                *p = Piece::Name {
                    ids: vec![(Space::Shared, relation)],
                    written: std::mem::take(written),
                    pun: false,
                };
            }
        }
    }

    /// Hold each name whose objects are no longer `live` by the keys `was` gives them, and each
    /// name of dropped objects one of whose keys `now` holds by the objects of those keys.
    pub(super) fn rebind(
        &mut self,
        was: &BTreeMap<Obj, ObjectKey>,
        now: &BTreeMap<ObjectKey, Obj>,
        live: &dyn Fn(Obj) -> bool,
    ) {
        for p in &mut self.0 {
            match p {
                Piece::Name { ids, written, pun } => {
                    if ids.iter().any(|&o| live(o) || o.1 < FIRST_LOG_ID) {
                        ids.retain(|&o| live(o) || o.1 < FIRST_LOG_ID);
                        continue;
                    }
                    *p = Piece::Dropped {
                        keys: ids.iter().filter_map(|o| was.get(o).cloned()).collect(),
                        written: std::mem::take(written),
                        pun: *pun,
                    };
                }
                Piece::Dropped { keys, written, pun } => {
                    let ids: Vec<Obj> = keys.iter().filter_map(|k| now.get(k).copied()).collect();
                    if !ids.is_empty() {
                        *p = Piece::Name {
                            ids,
                            written: std::mem::take(written),
                            pun: *pun,
                        };
                    }
                }
                Piece::Text(_) | Piece::Ahead { .. } => {}
            }
        }
    }
}
