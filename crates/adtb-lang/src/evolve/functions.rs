use super::*;

impl Source {
    /// Returns every function and aggregate of the user modules, in declaration order.
    pub fn functions(&self) -> Result<Vec<Function>, Vec<FileDiag>> {
        let schema = self.schema()?;
        let names = self.names()?;
        let deterministic = deterministic_functions(&schema);
        let mut out = Vec::new();
        for (file, f) in self.files()? {
            let Some(module) = schema.module_named(&f.module.text) else {
                continue;
            };
            let text = self.file_text(file);
            let names = &names[file.0 as usize];
            let term = |span: Span| {
                canonical_term(&text, span, names)
                    .map(|canonical| (text[span.range()].to_string(), canonical))
                    .map_err(|u| {
                        vec![FileDiag {
                            file,
                            diag: u.into(),
                        }]
                    })
            };
            let values = &schema.module(module).values;
            for d in &f.decls {
                let (id, name, result, det, code) = match d {
                    Decl::Fn(fd) => {
                        let Some(&ValueDef::Function(fid)) = values.get(&fd.name.text) else {
                            continue;
                        };
                        let code = match &fd.body {
                            ast::FnBody::Expr(e) => {
                                let (body, canonical) = term(e.span)?;
                                Code::Language { body, canonical }
                            }
                            ast::FnBody::Wasm { module, export } => Code::Wasm {
                                module: module.clone(),
                                export: export.clone(),
                            },
                            ast::FnBody::Host { name, version, .. } => Code::Host {
                                symbol: name.clone(),
                                version: version.clone(),
                            },
                        };
                        (
                            schema.ids().function_id(fid),
                            &fd.name,
                            fd.result.span,
                            deterministic[fid.0 as usize],
                            code,
                        )
                    }
                    Decl::Aggregate(a) => {
                        let Some(&ValueDef::Aggregate(i)) = values.get(&a.name.text) else {
                            continue;
                        };
                        let (body, canonical) =
                            term(after_eq(&text, a.result.span.end, a.span.end))?;
                        let id = schema.ids().aggregate_id(i as usize);
                        (
                            id,
                            &a.name,
                            a.result.span,
                            crate::resolve::aggregate_refs(&schema, i as usize)
                                .iter()
                                .all(|g| deterministic[g.0 as usize]),
                            Code::Aggregate { body, canonical },
                        )
                    }
                    _ => continue,
                };
                let Some(id) = id else { continue };
                out.push(Function {
                    id: id.raw(),
                    module: f.module.text.clone(),
                    name: name.text.clone(),
                    signature: text[name.span.end as usize..result.end as usize]
                        .trim()
                        .to_string(),
                    deterministic: det,
                    code,
                });
            }
        }
        Ok(out)
    }
}

/// Returns, for each function of `schema`, whether every function it calls, transitively, is
/// deterministic.
fn deterministic_functions(schema: &Schema) -> Vec<bool> {
    let calls: Vec<BTreeSet<FnId>> = (0..schema.functions.len() as u32)
        .map(|i| crate::resolve::function_refs(schema, FnId(i)))
        .collect();
    let mut det: Vec<bool> = schema
        .functions
        .iter()
        .map(|f| {
            !matches!(
                f.body,
                ast::FnBody::Host {
                    deterministic: false,
                    ..
                }
            )
        })
        .collect();
    let mut changed = true;
    while changed {
        changed = false;
        for (i, gs) in calls.iter().enumerate() {
            if det[i] && gs.iter().any(|g| !det[g.0 as usize]) {
                det[i] = false;
                changed = true;
            }
        }
    }
    det
}
