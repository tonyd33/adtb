use super::*;
use crate::canon;

impl Source {
    /// Returns the names of each declaration, by span in its file of [`Source::files`], declaration `i`
    /// at index `i`.
    pub fn names(&self) -> Result<Vec<Names>, Vec<FileDiag>> {
        let schema = self.schema()?;
        Ok(self.names_in(&schema))
    }

    /// Returns the names of each declaration as `names` does, and the declarations whose names
    /// wait: those that write a name this source does not resolve. `None` while the source does
    /// not build.
    fn names_built(&self) -> Option<(Vec<Names>, BTreeSet<usize>)> {
        let schema = self.schema().ok()?;
        let names = self.names_in(&schema);
        let waiting = (0..names.len())
            .filter(|&i| !names[i].all_resolve())
            .collect();
        Some((names, waiting))
    }

    fn names_in(&self, schema: &Schema) -> Vec<Names> {
        let mut out = vec![Names::default(); self.decls.len()];
        for (file, f) in self.files().expect("a source whose schema builds parses") {
            let module = schema
                .module_named(&f.module.text)
                .unwrap_or(schema::PRELUDE);
            for d in &f.decls {
                canon::decl_names(schema, module, d, &mut out[file.0 as usize]);
            }
        }
        for (file, span, r) in check::schema_refs(schema) {
            if let Some(names) = out.get_mut(file.0 as usize) {
                names.note(schema, span, &r);
            }
        }
        out
    }
}

impl Logged {
    /// Returns the names of this migration's text, by span.
    ///
    /// An operand the operator copies into a declaration (a declaration, a constructor, a field type, an
    /// arm, an index field, a default, a computed field) resolves where it was copied, in the source
    /// after the operator. When that source does not build, or the declaration writes a name of a
    /// type, relation variable, constructor or named set that source does not declare, the
    /// operand resolves in the first later source that builds and declares every such name the
    /// declaration writes, else in the migration's result. The fields of a dropped unique or index
    /// resolve in the clause they match before the operator. A fill or a routing resolves as the
    /// operator checks it.
    pub fn names(&self) -> Result<Names, Vec<Diag>> {
        self.names_with(false, None)
    }

    /// Returns the names of this migration's text as [`Logged::names`] does, and the source before
    /// each operator.
    pub(super) fn replayed(&self) -> Result<(Names, Vec<Source>), Vec<Diag>> {
        let mut sources = Vec::new();
        let names = self.names_with(false, Some(&mut sources))?;
        Ok((names, sources))
    }

    /// Returns the names of this migration's text as [`Logged::names`] does, with each operand
    /// that names an object tied to its catalog id as [`operands::tie_operands`] resolves it.
    pub(super) fn spelling(&self) -> Result<Names, Vec<Diag>> {
        self.names_with(true, None)
    }

    fn names_with(
        &self,
        operands: bool,
        mut sources: Option<&mut Vec<Source>>,
    ) -> Result<Names, Vec<Diag>> {
        let mut src = (*self.before).clone();
        src.version += 1;
        src.expecting(&self.module, &self.operators);
        src.marks = Some(Marks::default());
        let mut naming = Naming::default();
        for op in Listed::all(&self.operators) {
            let prior = src.clone();
            let noted = RefCell::new(Names::default());
            src.apply_noting(&self.module, op, &self.text, Some(&noted))?;
            let names = naming.noted(noted.into_inner());
            if operands {
                operands::tie_operands(&prior, &src, &self.module, &op, names);
            }
            naming.copied(&prior, &mut src, op.span)?;
            if let Some(out) = sources.as_deref_mut() {
                out.push(prior);
            }
        }
        naming.finish(&src)?;
        Ok(naming.into_names())
    }
}

/// The names of a migration's text, gathered as its operators apply one at a time to a source
/// that records its copies.
#[derive(Default)]
pub(super) struct Naming {
    names: Names,
    late: Vec<Range<usize>>,
    missing: Vec<Diag>,
}

impl Naming {
    /// Add the names an operator noted. Returns the names so far.
    pub(super) fn noted(&mut self, noted: Names) -> &mut Names {
        self.names.extend(noted);
        &mut self.names
    }

    /// Name the text of the migration the operator at `span` matched in `prior`, the source
    /// before it, and copied into `src`, the source after it.
    pub(super) fn copied(
        &mut self,
        prior: &Source,
        src: &mut Source,
        span: Span,
    ) -> Result<(), Vec<Diag>> {
        let marks = src.marks.as_mut().expect("a replay records its copies");
        let read = std::mem::take(&mut marks.read);
        let written: Vec<Mark> = marks
            .written
            .iter()
            .filter(|m| span.range().contains(&m.from.start))
            .cloned()
            .collect();
        if !read.is_empty() {
            let before = prior.names().map_err(unbuilt)?;
            copy_marked(&mut self.names, &before, prior, &read);
        }
        match src.names_built() {
            Some((after, waiting)) => {
                copy_marked(&mut self.names, &after, src, &written);
                self.late.extend(
                    written
                        .into_iter()
                        .filter(|m| waiting.contains(&m.decl))
                        .map(|m| m.from),
                );
                copy_late(
                    &mut self.names,
                    &after,
                    src,
                    &waiting,
                    &mut self.late,
                    &mut self.missing,
                );
            }
            None => self.late.extend(written.into_iter().map(|m| m.from)),
        }
        Ok(())
    }

    /// Name the copies still unnamed in `src`, the source the last operator leaves.
    ///
    /// Fail when the source does not build or a later operator rewrote a copy.
    pub(super) fn finish(&mut self, src: &Source) -> Result<(), Vec<Diag>> {
        if !self.late.is_empty() {
            let result = src.names().map_err(unbuilt)?;
            copy_late(
                &mut self.names,
                &result,
                src,
                &BTreeSet::new(),
                &mut self.late,
                &mut self.missing,
            );
        }
        if self.missing.is_empty() {
            Ok(())
        } else {
            Err(std::mem::take(&mut self.missing))
        }
    }

    pub(super) fn into_names(self) -> Names {
        self.names
    }
}

/// Copy into `names` the names `decl_names` holds at the copies in `src` of each range of
/// `late` that `names` does not name yet, keeping in `late` each range copied into a declaration
/// of `waiting`, and record in `missing` each range a later operator rewrote.
fn copy_late(
    names: &mut Names,
    decl_names: &[Names],
    src: &Source,
    waiting: &BTreeSet<usize>,
    late: &mut Vec<Range<usize>>,
    missing: &mut Vec<Diag>,
) {
    let marks = src.marks.as_ref().expect("a replay records its copies");
    let mut kept = Vec::new();
    for from in late.drain(..) {
        let pieces: Vec<Mark> = marks
            .written
            .iter()
            .filter(|m| from.start <= m.from.start && m.from.end <= from.end)
            .cloned()
            .collect();
        if pieces.is_empty() {
            missing.push(Diag::new(
                Span::new(from.start, from.end),
                "a later operator of the migration rewrites the declaration this operand was written into, so its names do not resolve",
            ));
        }
        if pieces.iter().any(|m| waiting.contains(&m.decl)) {
            kept.push(from);
            continue;
        }
        for m in &pieces {
            let at = header(&src.decls[m.decl].module).len() + m.at;
            names.copy_unnamed(&decl_names[m.decl], at..at + m.from.len(), m.from.start);
        }
    }
    *late = kept;
}

fn unbuilt(ds: Vec<FileDiag>) -> Vec<Diag> {
    ds.into_iter().map(|d| d.diag).collect()
}

/// Copy into `names` the names `decl_names` holds at each mark of a declaration of `src`.
fn copy_marked(names: &mut Names, decl_names: &[Names], src: &Source, marks: &[Mark]) {
    for m in marks {
        let at = header(&src.decls[m.decl].module).len() + m.at;
        names.copy(&decl_names[m.decl], at..at + m.from.len(), m.from.start);
    }
}
