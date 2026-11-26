use super::of_op::Inverting;
use super::*;
use crate::evolve::model::indexes::Part;
use crate::evolve::text::clause_ranges;

/// Returns the ids of the uniques and indexes model `m` holds on relation variable `r`.
fn index_ids(m: &Model, r: u64) -> BTreeSet<u64> {
    m.relations.get(&r).map_or_else(BTreeSet::new, |rel| {
        rel.indexes().filter_map(|(_, h)| h.id()).collect()
    })
}

/// Returns the one id `of` holds and `less` does not, `None` unless there is exactly one.
fn one_more(of: &BTreeSet<u64>, less: &BTreeSet<u64>) -> Option<u64> {
    let mut more = of.difference(less);
    match (more.next(), more.next()) {
        (Some(&id), None) => Some(id),
        _ => None,
    }
}

/// Move each step adding back a unique or index that names an object an operator before its own
/// removed to the end of the steps of the first such operator, whose steps apply after it.
pub(super) fn after_awaited(steps: &mut [Undoing]) {
    for i in 1..steps.len() {
        let mut k = 0;
        while k < steps[i].0.len() {
            let to = match &steps[i].0[k] {
                Inverse::RestoreIndex { awaits, .. } => {
                    (0..i).find(|&j| steps[j].1.iter().any(|(key, _)| awaits.contains(key)))
                }
                _ => None,
            };
            match to {
                Some(j) => {
                    let inv = steps[i].0.remove(k);
                    steps[j].0.push(inv);
                }
                None => k += 1,
            }
        }
    }
}

impl Inverting<'_> {
    /// Returns the step adding back unique or index `id` of relation variable `r` as the model
    /// before the operator holds it: an `add` operator over its elements on `r`, each name held
    /// by its id, a named set the migration dropped by its name, after which the step waits.
    pub(super) fn index_back(&self, r: u64, id: u64) -> R<Inverse> {
        let m = self.before_m();
        let held = m
            .relations
            .get(&r)
            .and_then(|rel| rel.indexes().map(|(_, h)| h).find(|h| h.id() == Some(id)))
            .ok_or_else(|| {
                vec![Diag::new(
                    self.op().span,
                    "the log does not hold the unique or index the operator removes",
                )]
            })?;
        let mut text = Term::from(match held.kind() {
            IndexKind::Unique => "add unique (",
            IndexKind::Plain => "add index (",
            IndexKind::Interval => "add interval index (",
        });
        let mut awaits = Vec::new();
        for (k, e) in held.elems().iter().enumerate() {
            if k > 0 {
                text.push(", ");
            }
            text.append(e.term().clone());
            awaits.extend(e.term().dropped_names().flat_map(|(keys, _)| keys.to_vec()));
        }
        text.push(") in ");
        text.append(self.relation_term(r)?);
        if let Some(parts) = held.parts() {
            let listed = parts.len() > 1;
            text.push(if listed { ".(" } else { "." });
            for (k, p) in parts.iter().enumerate() {
                if k > 0 {
                    text.push(" | ");
                }
                match p {
                    Part::Held { obj, written } => text.push_name(written, obj.0, Some(obj.1)),
                    Part::Gone { key, written, .. } => {
                        text.push(written.as_str());
                        awaits.push(key.clone());
                    }
                }
            }
            if listed {
                text.push(")");
            }
        }
        Ok(Inverse::RestoreIndex {
            index: id,
            text,
            awaits,
            of: self.of(),
        })
    }

    /// Returns the id of the one unique or index the operator adds to relation variable `r`.
    pub(super) fn index_added(&self, r: u64) -> R<u64> {
        let id = one_more(
            &index_ids(self.after_m(), r),
            &index_ids(self.before_m(), r),
        );
        Ok(id.ok_or_else(|| {
            vec![Diag::new(
                self.op().span,
                "the log does not hold the unique or index the operator adds",
            )]
        })?)
    }

    /// Returns the id of the one unique or index the operator removes from relation variable `r`.
    pub(super) fn index_removed(&self, r: u64) -> R<u64> {
        let id = one_more(
            &index_ids(self.before_m(), r),
            &index_ids(self.after_m(), r),
        );
        Ok(id.ok_or_else(|| {
            vec![Diag::new(
                self.op().span,
                "the log does not hold the unique or index the operator removes",
            )]
        })?)
    }

    /// Returns the steps adding back each unique and index of relation variable `r` as the
    /// model before the operator holds them, in clause order.
    pub(super) fn indexes_back(&self, r: u64) -> R<Vec<Inverse>> {
        let ids = match self.before_m().relations.get(&r) {
            Some(rel) => rel
                .indexes()
                .map(|(_, h)| h.id())
                .collect::<Option<Vec<u64>>>(),
            None => Some(Vec::new()),
        };
        let ids = ids.ok_or_else(|| {
            vec![Diag::new(
                self.op().span,
                "the log does not hold an id of a unique or index the operator removes",
            )]
        })?;
        ids.into_iter().map(|id| self.index_back(r, id)).collect()
    }

    /// Returns the text of relation declaration `i` of the source before the operator, as
    /// [`Inverting::decl`] does, without its unique and index clauses.
    pub(super) fn relation_without_indexes(&self, i: usize) -> R<Term> {
        let before = self.before();
        let Some(Decl::Relation(d)) = before.parsed(i) else {
            return Err(vec![Diag::new(
                self.op().span,
                "the log does not parse the relation the operator removes",
            )]
            .into());
        };
        let text = &before.decls[i].text;
        let ranges = clause_ranges(header(&before.decls[i].module).len(), &d);
        let mut out = Term::default();
        let mut from = 0;
        for (cl, r) in d.clauses.iter().zip(&ranges) {
            if matches!(
                cl,
                RelationClause::Unique(..) | RelationClause::Index { .. }
            ) {
                let cut = text[..r.start].trim_end().len();
                out.append(self.decl_range(i, from..cut)?);
                from = r.end;
            }
        }
        out.append(self.decl_range(i, from..text.len())?);
        Ok(out)
    }
}
