use super::ctor::RowTypes;
use super::*;
use crate::norm::{self, NormalQuery};

/// A function of one row, normalised against the schema it checks in.
#[derive(Debug)]
pub struct RowFn {
    pub schema: Schema,
    /// Its one parameter is the row.
    pub query: NormalQuery,
}

/// How an operator moves the rows of the relations over type `def` beyond what each field and
/// constructor it keeps carries by id.
///
/// Type and constructor indexes are those of the schema before the operator.
#[derive(Debug)]
pub enum RowMap {
    /// Field `field` of each row of constructors `ctors` takes the value of `fill` at the row.
    Fill {
        def: TypeId,
        ctors: Vec<u32>,
        field: String,
        fill: RowFn,
    },
    /// Field `field` of each row of constructors `ctors` keeps the value it had before.
    Copy {
        def: TypeId,
        ctors: Vec<u32>,
        field: String,
    },
    /// Each row of constructors `from` becomes the value of `routing` at the row, a constructor of
    /// `routing`'s schema named as the constructor of the schema after the operator it goes to.
    Route {
        def: TypeId,
        from: Vec<u32>,
        routing: RowFn,
    },
    /// Each row of constructors `from` becomes a row of constructor `into`, field
    /// `discriminator` holding the constructor of the discriminator's sum named as the row's, and
    /// every other field the row's field of the same name.
    Merge {
        def: TypeId,
        from: Vec<u32>,
        into: String,
        discriminator: String,
    },
    /// Each row of constructors `from` becomes a row of constructor `into`, whose one field holds
    /// the constructor of its sum named as the row's, with the row's fields after the header.
    Nest {
        def: TypeId,
        from: Vec<u32>,
        into: String,
    },
    /// Each row of constructor `from` becomes a row of the constructor named as the constructor
    /// its one field holds, with that value's fields after the header.
    Unnest { def: TypeId, from: u32 },
    /// Each field typed `Key` of relation `relation`, in any type, takes the new key of the row
    /// its old key names.
    Rekey { relation: RelId },
}

/// One step of applying an operator: the source after it, and how it moves rows.
#[derive(Debug)]
pub struct Step {
    pub after: Source,
    pub maps: Vec<RowMap>,
}

impl Source {
    /// Applies one operator of a migration of `module` whose text is `text`, as
    /// [`Source::apply`] does, and returns the steps it takes.
    ///
    /// Postconditions:
    /// - Each step's `maps` are stated against the `after` of the step before it, the first's
    ///   against this source as it was.
    /// - The last step's `after` is this source as the operator leaves it.
    pub fn steps(
        &mut self,
        module: &str,
        op: &Operator,
        text: &str,
    ) -> Result<Vec<Step>, Vec<Diag>> {
        let steps = RefCell::new(Vec::new());
        let before = self.clone();
        self.apply_recording(module, op, text, None, Some(&steps))?;
        let mut steps = steps.into_inner();
        if !matches!(op.kind, OperatorKind::Revert { .. }) {
            let maps = before.row_maps(module, op, text, self)?;
            steps.push(Step {
                after: self.clone(),
                maps,
            });
        }
        match steps.last_mut() {
            Some(last) => last.after = self.clone(),
            None => steps.push(Step {
                after: self.clone(),
                maps: Vec::new(),
            }),
        }
        Ok(steps)
    }

    /// Returns how `op`, applied to this source to give `after`, moves rows.
    pub(super) fn row_maps(
        &self,
        module: &str,
        op: &Operator,
        text: &str,
        after: &Source,
    ) -> Result<Vec<RowMap>, Vec<Diag>> {
        let mut src = self.clone();
        Cx {
            src: &mut src,
            module: module.to_string(),
            text,
            span: op.span,
            noted: None,
            copying: false,
            steps: None,
        }
        .row_maps(&op.kind, after)
    }
}

impl Cx<'_> {
    fn row_maps(&self, op: &OperatorKind, after: &Source) -> R<Vec<RowMap>> {
        match op {
            OperatorKind::AddField { path, fill, .. } => self.fill_map(path, fill, after),
            OperatorKind::MoveField {
                path, sel, fill, ..
            } => self.move_maps(path, sel.as_ref(), fill.as_ref()),
            OperatorKind::SplitConstructor {
                path, into, arms, ..
            } => {
                let schema = self.schema()?;
                let (_, _, def, c) = self.ctor_of(&schema, path)?;
                let products: Vec<String> = into.iter().map(|d| d.name.text.clone()).collect();
                let texts: Vec<Written> = into.iter().map(|d| self.ctor_def_text(d)).collect();
                let rt = self.routing_types(
                    &path.ty,
                    std::slice::from_ref(&path.ctor.text),
                    &products,
                    &texts,
                )?;
                Ok(vec![RowMap::Route {
                    def,
                    from: vec![c],
                    routing: self.row_fn(rt, RowBody::Match(arms, self.span))?,
                }])
            }
            OperatorKind::MergeConstructor {
                ty,
                sel,
                into,
                arms,
                ..
            } => {
                let schema = self.schema()?;
                let def = self.type_id(&schema, ty)?;
                let sources = self.sources(&schema, def, sel)?;
                let from = ctor_indexes(&schema, def, &sources);
                let m = into.name.text.clone();
                match arms {
                    Some(arms) => {
                        let rt = self.routing_types(
                            ty,
                            &sources,
                            std::slice::from_ref(&m),
                            &[self.ctor_def_text(into)],
                        )?;
                        Ok(vec![RowMap::Route {
                            def,
                            from,
                            routing: self.row_fn(rt, RowBody::Match(arms, self.span))?,
                        }])
                    }
                    None => {
                        let discriminator = self.discriminator(after, ty, &m, &sources)?;
                        Ok(vec![RowMap::Merge {
                            def,
                            from,
                            into: m,
                            discriminator,
                        }])
                    }
                }
            }
            OperatorKind::NestConstructor { ty, sel, into, .. } => {
                let schema = self.schema()?;
                let def = self.type_id(&schema, ty)?;
                let sources = self.sources(&schema, def, sel)?;
                Ok(vec![RowMap::Nest {
                    def,
                    from: ctor_indexes(&schema, def, &sources),
                    into: into.name.text.clone(),
                }])
            }
            OperatorKind::UnnestConstructor(path, _) => {
                let schema = self.schema()?;
                let (_, _, def, from) = self.ctor_of(&schema, path)?;
                Ok(vec![RowMap::Unnest { def, from }])
            }
            OperatorKind::Rekey { relation, .. } => {
                let schema = self.schema()?;
                Ok(vec![RowMap::Rekey {
                    relation: self.rel_id(&schema, relation)?,
                }])
            }
            _ => Ok(Vec::new()),
        }
    }

    /// The map of `add field path: _ fill fill`, whose result is `after`.
    fn fill_map(&self, path: &ast::FieldPath, fill: &Fill, after: &Source) -> R<Vec<RowMap>> {
        let (_, t) = self.type_decl(&path.ty)?;
        let part = self.part(&t, path)?;
        let (schema, program) = self.checked()?;
        let def = self.type_id(&schema, &path.ty)?;
        let after = after.schema().map_err(|ds| {
            ds.into_iter()
                .map(|d| Diag::new(self.span, d.diag.message))
                .collect::<Vec<_>>()
        })?;
        let ad = self.type_id(&after, &path.ty)?;
        let (ctors, field_ty) = match &part {
            Part::Header => (
                None,
                after
                    .ty(ad)
                    .header()
                    .iter()
                    .find(|f| f.name == path.field.text)
                    .map(|f| f.ty.clone()),
            ),
            Part::Ctor(c) => {
                let k = schema.ty(def).ctor_index(c).expect("the constructor");
                let ak = after.ty(ad).ctor_index(c).expect("the constructor");
                (
                    Some(vec![k]),
                    after
                        .ty(ad)
                        .field(ak, &path.field.text)
                        .map(|f| f.ty.clone()),
                )
            }
        };
        let Some(field_ty) = field_ty else {
            return self.err("the added field does not resolve");
        };
        let all: Vec<u32> = (0..schema.ty(def).ctors().len() as u32).collect();
        let row = Self::row_ty(&schema, def, ctors.clone());
        let module = schema
            .module_named(&self.module_of(&path.ty))
            .unwrap_or(schema::PRELUDE);
        let rt = RowTypes {
            schema,
            program,
            module,
            row,
            result: field_ty,
        };
        Ok(vec![RowMap::Fill {
            def,
            ctors: ctors.unwrap_or(all),
            field: path.field.text.clone(),
            fill: self.row_fn(rt, fill_body(fill, self.span))?,
        }])
    }

    /// The maps of `move field path to _ sel fill`.
    fn move_maps(
        &self,
        path: &ast::FieldPath,
        sel: Option<&CtorSel>,
        fill: Option<&Fill>,
    ) -> R<Vec<RowMap>> {
        let (_, t) = self.type_decl(&path.ty)?;
        let part = self.part(&t, path)?;
        let (schema, program) = self.checked()?;
        let def = self.type_id(&schema, &path.ty)?;
        let td = schema.ty(def);
        let field = path.field.text.clone();
        match (&part, sel) {
            (Part::Ctor(c), None) => {
                let k = td.ctor_index(c).expect("the constructor");
                let Some(field_ty) = td.field(k, &field).map(|f| f.ty.clone()) else {
                    return self.err(format!("`{c}` has no field `{field}`"));
                };
                let others: Vec<u32> = (0..td.ctors().len() as u32).filter(|&o| o != k).collect();
                let mut maps = vec![RowMap::Copy {
                    def,
                    ctors: vec![k],
                    field: field.clone(),
                }];
                if let (Some(fill), false) = (fill, others.is_empty()) {
                    let module = schema
                        .module_named(&self.module_of(&path.ty))
                        .unwrap_or(schema::PRELUDE);
                    let row = Self::row_ty(&schema, def, Some(others.clone()));
                    let rt = RowTypes {
                        schema,
                        program,
                        module,
                        row,
                        result: field_ty,
                    };
                    maps.push(RowMap::Fill {
                        def,
                        ctors: others,
                        field,
                        fill: self.row_fn(rt, fill_body(fill, self.span))?,
                    });
                }
                Ok(maps)
            }
            (Part::Header, Some(sel)) => {
                let sources = self.sources(&schema, def, sel)?;
                Ok(vec![RowMap::Copy {
                    def,
                    ctors: ctor_indexes(&schema, def, &sources),
                    field,
                }])
            }
            _ => Ok(Vec::new()),
        }
    }

    /// The field of constructor `m` of `ty` in `after` whose type is a declared nullary sum of
    /// exactly the constructors `sources`.
    fn discriminator(&self, after: &Source, ty: &QName, m: &str, sources: &[String]) -> R<String> {
        let after = after.schema().map_err(|ds| {
            ds.into_iter()
                .map(|d| Diag::new(self.span, d.diag.message))
                .collect::<Vec<_>>()
        })?;
        let ad = self.type_id(&after, ty)?;
        let td = after.ty(ad);
        let Some(k) = td.ctor_index(m) else {
            return self.err(format!("`{m}` is not a constructor of `{}`", td.name));
        };
        let wanted: BTreeSet<&str> = sources.iter().map(String::as_str).collect();
        td.ctors()[k as usize]
            .fields
            .iter()
            .find(|f| {
                let Ty::Data { def, .. } = &f.ty else {
                    return false;
                };
                let s = after.ty(*def);
                s.header().is_empty()
                    && s.ctors().iter().all(|c| c.fields.is_empty())
                    && s.ctors()
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<BTreeSet<_>>()
                        == wanted
            })
            .map(|f| f.name.clone())
            .map_or_else(|| self.err(format!("`{m}` has no discriminator field")), Ok)
    }

    /// Returns `body`, checked at `rt` and normalised.
    fn row_fn(&self, rt: RowTypes, body: RowBody) -> R<RowFn> {
        let q = check::row_query(
            &rt.schema,
            &rt.program,
            rt.module,
            &rt.row,
            body,
            Some(&rt.result),
        )
        .map_err(|d| vec![d])?;
        let query = norm::query(&rt.schema, &rt.program, &q).map_err(|e| {
            vec![Diag::new(
                self.span,
                format!("the fill does not normalise: {e:?}"),
            )]
        })?;
        Ok(RowFn {
            schema: rt.schema,
            query,
        })
    }
}

fn fill_body(fill: &Fill, span: Span) -> RowBody<'_> {
    match fill {
        Fill::Match(arms) => RowBody::Match(arms, span),
        Fill::Expr(e) => RowBody::Expr(e),
    }
}

fn ctor_indexes(schema: &Schema, def: TypeId, names: &[String]) -> Vec<u32> {
    let td = schema.ty(def);
    names.iter().filter_map(|n| td.ctor_index(n)).collect()
}
