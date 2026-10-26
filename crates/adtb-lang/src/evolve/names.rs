use super::*;
use crate::canon;

impl Source {
    /// Returns the names of each declaration, by span in its file of [`Source::files`], declaration `i`
    /// at index `i`.
    pub fn names(&self) -> Result<Vec<Names>, Vec<FileDiag>> {
        let schema = self.schema()?;
        Ok(self.names_in(&schema))
    }

    /// Returns the names of each declaration as `names` does, or `None` while a `Key r` type names
    /// a relation variable a later operator adds.
    fn names_settled(&self) -> Option<Vec<Names>> {
        let schema = self.schema().ok()?;
        schema.ahead.is_empty().then(|| self.names_in(&schema))
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
    /// after the operator, and when that source does not build, in the migration's result. The fields of
    /// a dropped unique or index resolve in the clause they match before the operator. A fill or a
    /// routing resolves as the operator checks it.
    pub fn names(&self) -> Result<Names, Vec<Diag>> {
        let mut src = (*self.before).clone();
        src.version += 1;
        src.expecting(&self.module, &self.operators);
        src.marks = Some(Marks::default());
        let mut names = Names::default();
        let mut late = Vec::new();
        for op in &self.operators {
            let prior = src.clone();
            let noted = RefCell::new(Names::default());
            src.apply_noting(&self.module, op, &self.text, Some(&noted))?;
            names.extend(noted.into_inner());
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
            match src.names_settled() {
                Some(after) => copy_marked(&mut names, &after, &src, &written),
                None => late.extend(written.into_iter().map(|m| m.from)),
            }
        }
        if late.is_empty() {
            return Ok(names);
        }
        let result = src.names().map_err(unbuilt)?;
        let marks = src.marks.as_ref().expect("a replay records its copies");
        let mut missing = Vec::new();
        for from in late {
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
            copy_marked(&mut names, &result, &src, &pieces);
        }
        if missing.is_empty() {
            Ok(names)
        } else {
            Err(missing)
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
