use super::*;
use crate::canon;

impl Source {
    /// Returns the names of each declaration, by span in its file of [`Source::files`], declaration `i`
    /// at index `i`.
    pub fn names(&self) -> Result<Vec<Names>, Vec<FileDiag>> {
        let schema = self.schema()?;
        Ok(self.names_in(&schema))
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
    /// operator checks it. Any other operand naming an object resolves in the source before the
    /// operator, and the field an `add field` or `add computed` declares in the source after it.
    pub fn names(&self) -> Replaying<Names> {
        Ok(model::replay(self)?.1?)
    }

    /// Returns the source before this migration's first operator and after each.
    pub fn states(&self) -> Replaying<Vec<Source>> {
        Ok(model::replay(self)?.0)
    }

    /// Returns the source before operator `k` of this migration and the steps operator `k` takes
    /// from it, as [`Recorded::steps`] gives them.
    ///
    /// Fail with the refusals of the operators that do not apply, or why the steps fail.
    pub fn stepped(&self, k: usize) -> Replaying<(Source, Vec<Step>)> {
        let (mut states, _, mut got) = model::replay(self)?;
        let m = model::logged_file(self);
        let steps = got.op_steps(&m).into_iter().nth(k);
        let steps = steps.ok_or_else(|| {
            vec![Diag::new(
                self.operators[k].span,
                "an earlier operator's steps fail",
            )]
        })??;
        Ok((states.swap_remove(k), steps))
    }
}
