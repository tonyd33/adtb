use std::ops::Range;

use std::collections::BTreeMap;

use super::{Model, Obj};
use crate::canon::Names;
use crate::evolve::Written;
use crate::evolve::model::inverse::{Late, Recreated, dropped, word};
use crate::ids::{FIRST_LOG_ID, ObjectKey, Space};
use crate::schema::Adder;

/// Text written in the language, each name in it held as the objects it resolves to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::evolve) struct Term(Vec<Piece>);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Text(String),
    /// Text copied from the text of the migration being applied, at `from`.
    Copy {
        text: String,
        from: Range<usize>,
    },
    /// Text of the arm `key` an operator wrote, held apart until the declarations build.
    Arm {
        key: u64,
        text: String,
    },
    /// A name resolving to `ids`, as `written`; `pun` when it also stands for a variable of
    /// that name.
    Name {
        ids: Vec<Obj>,
        written: String,
        pun: bool,
    },
    /// The name of the relation variable of the model's [`super::AheadM`] that `by` adds, before
    /// `by` adds it.
    Added {
        by: Adder,
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
            | Piece::Copy { text: s, .. }
            | Piece::Arm { text: s, .. }
            | Piece::Dropped { written: s, .. } => s.clone(),
            Piece::Added { by, written } => model
                .ahead
                .iter()
                .find(|a| a.by == *by)
                .map_or(written, |a| &a.name)
                .clone(),
            Piece::Name { ids, written, pun } => {
                let now = match ids.iter().find_map(|&o| model.name(o)) {
                    Some(now) => now,
                    None => written,
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
    /// and each name `ahead` holds by what adds its relation variable.
    pub(super) fn lift(
        text: &str,
        range: Range<usize>,
        names: &Names,
        ahead: &[(Range<usize>, Adder)],
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
                        let piece = Piece::Added {
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

    /// Returns the text `s` of arm `key`.
    pub(super) fn arm(key: u64, s: &str) -> Term {
        Term(vec![Piece::Arm {
            key,
            text: s.to_string(),
        }])
    }

    /// Returns each arm of the term with the range it takes in the term as `model` prints it,
    /// from its first piece through its last.
    pub(super) fn arm_ranges(&self, model: &Model) -> Vec<(u64, Range<usize>)> {
        let mut at = 0;
        let mut out: Vec<(u64, Range<usize>)> = Vec::new();
        for p in &self.0 {
            let n = p.print(model).len();
            if let Piece::Arm { key, .. } = p {
                match out.iter_mut().find(|(k, _)| k == key) {
                    Some((_, r)) => r.end = at + n,
                    None => out.push((*key, at..at + n)),
                }
            }
            at += n;
        }
        out
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

    /// Returns each name of the term as `model` names its objects now, with the objects it holds.
    pub(super) fn named(&self, model: &Model) -> Vec<(String, Vec<Obj>)> {
        self.0
            .iter()
            .filter_map(|p| match p {
                Piece::Name { ids, written, .. } => {
                    let now = ids.iter().find_map(|&o| model.name(o)).unwrap_or(written);
                    Some((now.to_string(), ids.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// Returns the object and the spelling of a term that is one name of one object alone.
    pub(super) fn as_name(&self) -> Option<(Obj, &str)> {
        match self.0.as_slice() {
            [
                Piece::Name {
                    ids,
                    written,
                    pun: false,
                },
            ] => match ids.as_slice() {
                [o] => Some((*o, written.as_str())),
                _ => None,
            },
            _ => None,
        }
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
                Piece::Text(_)
                | Piece::Copy { .. }
                | Piece::Arm { .. }
                | Piece::Added { .. }
                | Piece::Dropped { .. } => &[],
            })
            .copied()
    }

    /// Returns the objects each name of the term holds, a name at a time.
    pub(super) fn held_names(&self) -> Vec<&[Obj]> {
        self.0
            .iter()
            .filter_map(|p| match p {
                Piece::Name { ids, .. } => Some(ids.as_slice()),
                _ => None,
            })
            .collect()
    }

    /// Hold each name of an object `map` maps by the object it maps it to.
    pub(super) fn remap(&mut self, map: &BTreeMap<Obj, Obj>) {
        for p in &mut self.0 {
            if let Piece::Name { ids, .. } = p {
                for o in ids.iter_mut() {
                    if let Some(&to) = map.get(o) {
                        *o = to;
                    }
                }
                ids.dedup();
            }
        }
    }

    /// Hold each name of the relation variable `by` adds by `relation`, its id.
    pub(super) fn added(&mut self, by: Adder, relation: u64) {
        for p in &mut self.0 {
            if let Piece::Added { by: k, written } = p
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
                Piece::Text(_) | Piece::Copy { .. } | Piece::Arm { .. } | Piece::Added { .. } => {}
            }
        }
    }

    /// Append text `s`.
    pub(in crate::evolve) fn push(&mut self, s: impl Into<Written>) {
        let w: Written = s.into();
        let mut at = 0;
        for (o, from) in &w.copies {
            if *o > at {
                self.0.push(Piece::Text(w.text[at..*o].to_string()));
            }
            let end = o + from.len();
            self.0.push(Piece::Copy {
                text: w.text[*o..end].to_string(),
                from: from.clone(),
            });
            at = end;
        }
        if at < w.text.len() {
            self.0.push(Piece::Text(w.text[at..].to_string()));
        }
    }

    /// Append name `written`, held by the id `id` of `space` when it is `Some`.
    pub(in crate::evolve) fn push_name(&mut self, written: &str, space: Space, id: Option<u64>) {
        self.0.push(Piece::Name {
            ids: id.map(|id| (space, id)).into_iter().collect(),
            written: written.to_string(),
            pun: false,
        });
    }

    pub(in crate::evolve) fn append(&mut self, other: Term) {
        self.0.extend(other.0);
    }

    pub(in crate::evolve) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Append `text[range]`, each name in it that `names` resolves held by its ids, where
    /// `names` gives the names of `text` at offsets `at` past those of `text`.
    pub(in crate::evolve) fn copy(
        &mut self,
        text: &str,
        range: Range<usize>,
        names: &Names,
        at: usize,
    ) {
        let mut from = range.start;
        for (r, ids, pun) in names.ids_within(range.start + at..range.end + at) {
            let r = r.start - at..r.end - at;
            if r.start < from {
                continue;
            }
            if r.start > from {
                self.0.push(Piece::Text(text[from..r.start].to_string()));
            }
            self.0.push(Piece::Name {
                ids: ids.iter().copied().collect(),
                written: text[r.clone()].to_string(),
                pun,
            });
            from = r.end;
        }
        if from < range.end {
            self.0.push(Piece::Text(text[from..range.end].to_string()));
        }
    }

    /// Returns the term with its first name held by `ids` and written `written`, `None` when it
    /// holds no name.
    pub(in crate::evolve) fn with_head(mut self, ids: Vec<Obj>, written: &str) -> Option<Term> {
        let head = self
            .0
            .iter_mut()
            .find(|p| matches!(p, Piece::Name { .. }))?;
        *head = Piece::Name {
            ids,
            written: written.to_string(),
            pun: false,
        };
        Some(self)
    }

    /// Returns the term with each name held by id `id` of `space` also held by id `also` of it.
    pub(in crate::evolve) fn aliased(mut self, space: Space, id: u64, also: u64) -> Term {
        for p in &mut self.0 {
            if let Piece::Name { ids, .. } = p
                && ids.contains(&(space, id))
                && !ids.contains(&(space, also))
            {
                ids.push((space, also));
                ids.sort_unstable();
            }
        }
        self
    }

    /// Returns the term with each id a step of a revert created again replaced by the id `again`
    /// maps it to.
    pub(in crate::evolve) fn mapped(mut self, again: &Recreated) -> Term {
        for p in &mut self.0 {
            if let Piece::Name { ids, .. } = p {
                for (s, id) in ids.iter_mut() {
                    if let Some(&to) = again.get(&(*s, *id)) {
                        *id = to;
                    }
                }
                ids.sort_unstable();
                ids.dedup();
            }
        }
        self
    }

    /// Returns the term with each name spelled as `now` names its objects, a name none of whose
    /// objects `now` or `late` holds kept as written, and a pun whose name changes written as
    /// `name: variable`.
    ///
    /// Fail with the refusal of the inverse of operator text `of` when a name stands for an object
    /// `late` holds and `now` lacks, which a later migration dropped, or when the objects of one
    /// name now have different names.
    pub(in crate::evolve) fn spelled(
        &self,
        now: &BTreeMap<Obj, &str>,
        late: &Late,
        of: &str,
    ) -> Result<Written, String> {
        let mut out = Written::default();
        for p in &self.0 {
            match p {
                Piece::Copy { text, from } => out.append(Written {
                    text: text.clone(),
                    copies: vec![(0, from.clone())],
                }),
                Piece::Text(s)
                | Piece::Arm { text: s, .. }
                | Piece::Added { written: s, .. }
                | Piece::Dropped { written: s, .. } => out.push(s),
                Piece::Name { ids, written, pun } => {
                    if let Some(kind) = ids
                        .iter()
                        .filter(|i| !now.contains_key(i))
                        .find_map(|i| late.get(i))
                    {
                        return Err(dropped(of, word(*kind)));
                    }
                    let named: std::collections::BTreeSet<&str> =
                        ids.iter().filter_map(|i| now.get(i)).copied().collect();
                    let mut named = named.into_iter();
                    match (named.next(), named.next()) {
                        (Some(n), None) if *pun && n != written => {
                            out.push(&format!("{n}: {written}"));
                        }
                        (Some(n), None) => out.push(n),
                        (Some(_), Some(_)) => {
                            return Err(format!(
                                "in the inverse of `{of}`, the name `{written}` stood for objects a later migration renamed apart"
                            ));
                        }
                        (None, _) => out.push(written),
                    }
                }
            }
        }
        Ok(out)
    }
}

impl<T: Into<Written>> From<T> for Term {
    fn from(text: T) -> Term {
        let mut t = Term::default();
        t.push(text);
        t
    }
}
