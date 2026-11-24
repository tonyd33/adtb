use super::*;
use crate::ids::{KeyKind, Space};

/// A text an inverse applies, each name of a catalog object in it held as the object's ids.
#[derive(Clone, Default)]
pub(in crate::evolve) struct Spelled {
    pieces: Vec<Piece>,
}

impl std::fmt::Debug for Spelled {
    /// Show the tokens of the text, each name as written with its ids.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut shown: Vec<String> = Vec::new();
        let mut text = String::new();
        let flush = |text: &mut String, shown: &mut Vec<String>| {
            shown.extend(tokens(text).into_iter().map(|(_, r)| text[r].to_string()));
            text.clear();
        };
        for p in &self.pieces {
            match p {
                Piece::Text(w) => text.push_str(&w.text),
                Piece::Name { written, ids, pun } => {
                    flush(&mut text, &mut shown);
                    shown.push(format!("{written}{ids:?}{}", if *pun { "pun" } else { "" }));
                }
            }
        }
        flush(&mut text, &mut shown);
        write!(f, "{:?}", shown.join(" "))
    }
}

#[derive(Clone, Debug)]
enum Piece {
    Text(Written),
    Name {
        written: String,
        ids: BTreeSet<(Space, u64)>,
        pun: bool,
    },
}

/// The kind of each object the source after a reverted migration holds, by the space of its id
/// and the id.
pub(in crate::evolve) type Late = BTreeMap<(Space, u64), KeyKind>;

/// Returns the kind of each object `src` holds, by the space of its id and the id.
pub(super) fn late(src: &Source) -> Late {
    src.ids
        .iter()
        .map(|(k, &id)| ((k.kind().space(), id), k.kind()))
        .collect()
}

/// Returns the name of each named object `src` holds, by the space of its id and the id.
pub(super) fn spellings(src: &Source) -> BTreeMap<(Space, u64), &str> {
    src.ids
        .iter()
        .filter_map(|(k, &id)| Some(((k.kind().space(), id), k.name()?)))
        .collect()
}

/// Returns the word a refusal names an object of kind `kind` by.
pub(in crate::evolve) fn word(kind: KeyKind) -> &'static str {
    match kind {
        KeyKind::Type => "type",
        KeyKind::Ctor => "constructor",
        KeyKind::Relation => "relation",
        KeyKind::Field => "field",
        KeyKind::Set => "set",
        KeyKind::Function => "function",
        KeyKind::Constraint => "constraint",
        KeyKind::Computed => "computed field",
        KeyKind::Key
        | KeyKind::Unique
        | KeyKind::Index
        | KeyKind::IntervalIndex
        | KeyKind::Implied => "index",
        KeyKind::Generated | KeyKind::Default => "default",
    }
}

impl<T: Into<Written>> From<T> for Spelled {
    fn from(text: T) -> Spelled {
        let mut s = Spelled::default();
        s.push(text);
        s
    }
}

impl Spelled {
    pub(super) fn push(&mut self, text: impl Into<Written>) {
        self.pieces.push(Piece::Text(text.into()));
    }

    pub(super) fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// Append name `written`, held by the id `id` of `space` when it is `Some`.
    pub(super) fn name(&mut self, written: &str, space: Space, id: Option<u64>) {
        self.pieces.push(Piece::Name {
            written: written.to_string(),
            ids: id.map(|id| (space, id)).into_iter().collect(),
            pun: false,
        });
    }

    /// Returns the text with each name held by id `id` of `space` also held by id `also` of it.
    pub(super) fn aliased(mut self, space: Space, id: u64, also: u64) -> Spelled {
        for p in &mut self.pieces {
            if let Piece::Name { ids, .. } = p
                && ids.contains(&(space, id))
            {
                ids.insert((space, also));
            }
        }
        self
    }

    pub(super) fn append(&mut self, other: Spelled) {
        self.pieces.extend(other.pieces);
    }

    /// Append `text[range]`, each name in it that `names` resolves held by its ids, where
    /// `names` gives the names of `text` at offsets `at` past those of `text`.
    pub(super) fn copy(&mut self, text: &str, range: Range<usize>, names: &Names, at: usize) {
        let mut from = range.start;
        for (r, ids, pun) in names.ids_within(range.start + at..range.end + at) {
            let r = r.start - at..r.end - at;
            if r.start < from {
                continue;
            }
            self.push(&text[from..r.start]);
            self.pieces.push(Piece::Name {
                written: text[r.clone()].to_string(),
                ids: ids.clone(),
                pun,
            });
            from = r.end;
        }
        self.push(&text[from..range.end]);
    }

    /// Returns the text with each id a step of the revert created again replaced by the id
    /// `again` maps it to.
    pub(in crate::evolve) fn mapped(self, again: &Recreated) -> Spelled {
        let pieces = self
            .pieces
            .into_iter()
            .map(|p| match p {
                Piece::Name { written, ids, pun } => Piece::Name {
                    written,
                    pun,
                    ids: ids
                        .into_iter()
                        .map(|(s, id)| (s, again.get(&(s, id)).copied().unwrap_or(id)))
                        .collect(),
                },
                text => text,
            })
            .collect();
        Spelled { pieces }
    }

    /// Returns the text with each name spelled as `now` names its objects, a name none of whose
    /// objects `now` or `late` holds kept as written, and a pun whose name changes written as
    /// `name: variable`.
    ///
    /// Fail with the refusal of the inverse of operator text `of` when a name stands for an object
    /// `late` holds and `now` lacks, which a later migration dropped, or when the objects of one
    /// name now have different names.
    pub(in crate::evolve) fn spelled(
        &self,
        now: &BTreeMap<(Space, u64), &str>,
        late: &Late,
        of: &str,
    ) -> Result<Written, String> {
        let mut out = Written::default();
        for p in &self.pieces {
            match p {
                Piece::Text(w) => out.append(w.clone()),
                Piece::Name { written, ids, pun } => {
                    if let Some(kind) = ids
                        .iter()
                        .filter(|i| !now.contains_key(i))
                        .find_map(|i| late.get(i))
                    {
                        return Err(dropped(of, word(*kind)));
                    }
                    let named: BTreeSet<&str> =
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
