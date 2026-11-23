use super::*;

impl Source {
    /// Returns every function and aggregate of the user modules, in declaration order.
    pub fn functions(&self) -> Result<Vec<Function>, Vec<FileDiag>> {
        let schema = self.schema()?;
        let names = self.names()?;
        let checked = check::check_schema(&schema)?;
        let judge = crate::capability::Judge::new(&checked);
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
                            judge.deterministic_function(fid),
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
                            judge.deterministic_aggregate(i as usize),
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

/// Returns the span from the first token after the first `=` at or after `from` to `end`.
fn after_eq(text: &str, from: u32, end: u32) -> Span {
    let toks = tokens(&text[from as usize..end as usize]);
    let at = toks
        .iter()
        .position(|(t, _)| *t == Tok::Eq)
        .and_then(|i| toks.get(i + 1))
        .map_or(end as usize, |(_, r)| from as usize + r.start);
    Span::new(at, end as usize)
}
