use super::*;

impl Cx<'_> {
    pub(super) fn err<T, E: From<Vec<Diag>>>(&self, msg: impl Into<String>) -> Result<T, E> {
        Err(vec![Diag::new(self.span, msg)].into())
    }

    pub(super) fn err_at<T, E: From<Vec<Diag>>>(
        &self,
        span: Span,
        msg: impl Into<String>,
    ) -> Result<T, E> {
        Err(vec![Diag::new(span, msg)].into())
    }

    pub(super) fn slice(&self, span: Span) -> &str {
        &self.text[span.range()]
    }

    /// Returns the text at `span` of the migration, as a copy of it.
    pub(super) fn copy(&self, span: Span) -> Written {
        self.copy_range(span.range())
    }

    pub(super) fn copy_range(&self, r: Range<usize>) -> Written {
        Written {
            text: self.text[r.clone()].to_string(),
            copies: if self.copying {
                vec![(0, r)]
            } else {
                Vec::new()
            },
        }
    }

    pub(super) fn module_of(&self, q: &QName) -> String {
        q.module
            .as_ref()
            .map_or_else(|| self.module.clone(), |m| m.text.clone())
    }

    pub(super) fn type_id(&self, schema: &Schema, q: &QName) -> R<TypeId> {
        let m = self.module_of(q);
        schema
            .module_named(&m)
            .and_then(|mid| schema.module(mid).types.get(&q.name.text).copied())
            .map_or_else(
                || {
                    self.err_at(
                        q.span(),
                        format!("module `{m}` declares no type `{}`", q.name.text),
                    )
                },
                Ok,
            )
    }

    /// The relation variables of type `def`, by module and name.
    pub(super) fn relations_of(schema: &Schema, def: TypeId) -> Vec<(RelId, String, String)> {
        schema
            .relations
            .iter()
            .enumerate()
            .filter(|(_, r)| r.elem_def == def)
            .map(|(i, r)| {
                (
                    RelId(i as u32),
                    schema.module(r.module).name.clone(),
                    r.name.clone(),
                )
            })
            .collect()
    }
}
