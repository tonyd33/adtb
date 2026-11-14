use super::*;
use crate::canon;

impl Source {
    /// Returns the names of each declaration, by span in its file of [`Source::files`], declaration `i`
    /// at index `i`.
    pub fn names(&self) -> Result<Vec<Names>, Vec<FileDiag>> {
        let schema = self.schema()?;
        Ok(self.names_in(&schema))
    }

    /// Returns the names of each declaration as `names` does, and whether they are settled: no
    /// `Key r` type names a relation variable a later operator adds. `None` while the source does
    /// not build.
    fn names_built(&self) -> Option<(Vec<Names>, bool)> {
        let schema = self.schema().ok()?;
        Some((self.names_in(&schema), schema.ahead.is_empty()))
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
    /// after the operator; a name that source does not resolve, because it does not build or names a
    /// relation variable a later operator adds, resolves in the first later source that builds with
    /// none, else in the migration's result. The fields of
    /// a dropped unique or index resolve in the clause they match before the operator. A fill or a
    /// routing resolves as the operator checks it.
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
        let mut names = Names::default();
        let mut late = Vec::new();
        let mut missing = Vec::new();
        for op in &self.operators {
            let prior = src.clone();
            let noted = RefCell::new(Names::default());
            src.apply_noting(&self.module, op, &self.text, Some(&noted))?;
            names.extend(noted.into_inner());
            if operands {
                operands::tie_operands(&prior, &src, &self.module, op, &mut names);
            }
            let marks = src.marks.as_mut().expect("a replay records its copies");
            let read = std::mem::take(&mut marks.read);
            let written: Vec<Mark> = marks
                .written
                .iter()
                .filter(|m| op.span.range().contains(&m.from.start))
                .cloned()
                .collect();
            if !read.is_empty() {
                let before = prior.names().map_err(unbuilt)?;
                copy_marked(&mut names, &before, &prior, &read);
            }
            match src.names_built() {
                Some((after, true)) => {
                    copy_marked(&mut names, &after, &src, &written);
                    copy_late(&mut names, &after, &src, &mut late, &mut missing);
                }
                Some((after, false)) => {
                    copy_marked(&mut names, &after, &src, &written);
                    late.extend(written.into_iter().map(|m| m.from));
                }
                None => late.extend(written.into_iter().map(|m| m.from)),
            }
            if let Some(out) = sources.as_deref_mut() {
                out.push(prior);
            }
        }
        if !late.is_empty() {
            let result = src.names().map_err(unbuilt)?;
            copy_late(&mut names, &result, &src, &mut late, &mut missing);
        }
        if missing.is_empty() {
            Ok(names)
        } else {
            Err(missing)
        }
    }
}

/// Copy into `names` the names `decl_names` holds at the copies in `src` of each range of
/// `late` that `names` does not name yet, emptying `late`, and record in `missing` each range a
/// later operator rewrote.
fn copy_late(
    names: &mut Names,
    decl_names: &[Names],
    src: &Source,
    late: &mut Vec<Range<usize>>,
    missing: &mut Vec<Diag>,
) {
    let marks = src.marks.as_ref().expect("a replay records its copies");
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
        for m in &pieces {
            let at = header(&src.decls[m.decl].module).len() + m.at;
            names.copy_unnamed(&decl_names[m.decl], at..at + m.from.len(), m.from.start);
        }
    }
}

fn unbuilt(ds: Vec<FileDiag>) -> Vec<Diag> {
    ds.into_iter().map(|d| d.diag).collect()
}

/// Copies into `names` the names `decl_names` holds at each mark of a declaration of `src`.
fn copy_marked(names: &mut Names, decl_names: &[Names], src: &Source, marks: &[Mark]) {
    for m in marks {
        let at = header(&src.decls[m.decl].module).len() + m.at;
        names.copy(&decl_names[m.decl], at..at + m.from.len(), m.from.start);
    }
}
