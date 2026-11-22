use super::*;

impl Cx<'_> {
    pub(super) fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(vec![Diag::new(self.span, msg)])
    }

    pub(super) fn err_at<T>(&self, span: Span, msg: impl Into<String>) -> R<T> {
        Err(vec![Diag::new(span, msg)])
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

    pub(super) fn schema(&self) -> R<Schema> {
        self.src.schema().map_err(|ds| {
            ds.into_iter()
                .map(|d| d.diag.restated(self.span, |m| m))
                .collect()
        })
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

    pub(super) fn rel_id(&self, schema: &Schema, q: &QName) -> R<RelId> {
        let m = self.module_of(q);
        match schema
            .module_named(&m)
            .and_then(|mid| schema.module(mid).values.get(&q.name.text))
        {
            Some(ValueDef::Relation(r)) => Ok(*r),
            _ => self.err_at(
                q.span(),
                format!("module `{m}` declares no relation `{}`", q.name.text),
            ),
        }
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

    /// The state a lifecycle operator names, and how a refusal names it.
    pub(super) fn stage_key(&self, t: &CtorTarget) -> R<(StateOf, String)> {
        let schema = self.schema()?;
        match t {
            CtorTarget::Relation(rr) => {
                let rel = self.rel_id(&schema, &rr.relation)?;
                let r = schema.relation(rel);
                let def = schema.ty(r.elem_def);
                let named = match &rr.sel {
                    Some(CtorSel::One(c)) => def.ctor_named(&c.text).map(|ctor| (c, ctor)),
                    _ => None,
                };
                let Some((c, ctor)) = named else {
                    return self.err_at(
                        rr.span,
                        "a lifecycle operator names one constructor: `r.C` or `T.C`",
                    );
                };
                let ids = schema.ids();
                Ok((
                    StateOf::Relation(held_id(ids.relation_id(rel)), held_id(ids.ctor_id(ctor))),
                    format!("{}.{}", r.name, c.text),
                ))
            }
            CtorTarget::Type(p) => {
                let def = self.type_id(&schema, &p.ty)?;
                let t = schema.ty(def);
                let Some(ctor) = t.ctor_named(&p.ctor.text) else {
                    return self.err_at(
                        p.ctor.span,
                        format!("`{}` is not a constructor of `{}`", p.ctor.text, t.name),
                    );
                };
                Ok((
                    StateOf::Value(held_id(schema.ids().ctor_id(ctor))),
                    format!("{}.{}", t.name, p.ctor.text),
                ))
            }
        }
    }
}
