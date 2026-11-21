use super::*;
use crate::canon::decl_refs;

/// Returns the indexes of the declarations of `file` in an order that adds each after the
/// declarations it needs, `others` being the files of the other modules.
///
/// Precondition: `file` is its module's only file.
///
/// Postconditions:
/// - On `Ok`, every index appears once. A declaration follows each declaration of `file` whose
///   object it names, as the schema builder and the checker resolve its names, except that a type
///   need not follow the relation variable or the named sets and constructors a `Key r` field type
///   of it names when they in turn need the type. Each declaration in `file` order is placed after
///   those it needs that are not placed yet.
/// - On `Err`, the diagnostics are those of building `file` with `others`, else of checking that
///   schema, else one at a declaration of `file` that needs itself through others.
pub fn declaration_order(
    file: &ast::SchemaFile,
    others: &[ast::SchemaFile],
) -> Result<Vec<usize>, Vec<FileDiag>> {
    let mut files = vec![(FileId(0), file.clone())];
    files.extend((1..).zip(others).map(|(i, f)| (FileId(i), f.clone())));
    let schema = crate::schema::build(&files)?;
    check_schema(&schema)?;
    let module = schema
        .module_named(&file.module.text)
        .expect("a built file declares its module");
    let decls = &file.decls;
    let mut needs: Vec<Vec<(usize, bool)>> = vec![Vec::new(); decls.len()];
    let mut need = |at: usize, r: &Ref, soft: bool| {
        let Some(j) =
            declared_at(&schema, r).and_then(|span| decls.iter().position(|d| d.span() == span))
        else {
            return;
        };
        if j != at {
            needs[at].push((j, soft));
        }
    };
    for (i, d) in decls.iter().enumerate() {
        let keys = key_spans(d);
        for (span, r) in decl_refs(&schema, module, d) {
            let soft = keys
                .iter()
                .any(|k| k.start <= span.start && span.end <= k.end);
            need(i, &r, soft);
        }
    }
    for (f, span, r) in schema_refs(&schema) {
        let holder = decls.iter().position(|d| {
            let at = d.span();
            at.start <= span.start && span.end <= at.end
        });
        if let (FileId(0), Some(i)) = (f, holder) {
            need(i, &r, false);
        }
    }
    let reach: Vec<Vec<bool>> = (0..decls.len()).map(|i| reachable(i, &needs)).collect();
    let kept: Vec<Vec<usize>> = needs
        .iter()
        .enumerate()
        .map(|(i, ns)| {
            ns.iter()
                .filter(|&&(j, soft)| !(soft && reach[j][i]))
                .map(|&(j, _)| j)
                .collect()
        })
        .collect();
    let mut state = vec![Visit::New; decls.len()];
    let mut out = Vec::with_capacity(decls.len());
    for i in 0..decls.len() {
        place(i, &kept, &mut state, &mut out).map_err(|j| {
            vec![FileDiag {
                file: FileId(0),
                diag: diag(
                    decls[j].span(),
                    "the declaration needs itself through the declarations it names",
                ),
            }]
        })?;
    }
    Ok(out)
}

/// Returns the migration that adds the module of `file`, whose text is `text`, then its
/// declarations in the order [`declaration_order`] gives, each after its verb.
pub fn schema_migration(
    text: &str,
    file: &ast::SchemaFile,
    others: &[ast::SchemaFile],
) -> Result<String, Vec<FileDiag>> {
    let module = &file.module.text;
    let mut out = format!("module {module}\nadd module {module}\n");
    for i in declaration_order(file, others)? {
        let d = &file.decls[i];
        out.push_str(match d {
            ast::Decl::Fn(_) | ast::Decl::Aggregate(_) => "define ",
            ast::Decl::Store(_) => "",
            _ => "add ",
        });
        out.push_str(&text[d.span().range()]);
        out.push('\n');
    }
    Ok(out)
}

/// Returns the spans of the `Key r` relation references in the field types of type declaration
/// `d`, none for another declaration.
fn key_spans(d: &ast::Decl) -> Vec<Span> {
    fn walk(t: &ast::Type, out: &mut Vec<Span>) {
        match &t.kind {
            ast::TypeKind::Key(_, r) => out.push(r.span),
            ast::TypeKind::Named { args, .. } => {
                for a in args {
                    if let ast::TypeArg::Type(t) = a {
                        walk(t, out);
                    }
                }
            }
            ast::TypeKind::Record(fs) => fs.iter().for_each(|f| walk(&f.ty, out)),
            ast::TypeKind::Sum(cs) => cs
                .iter()
                .flat_map(|c| c.record.iter().flatten())
                .for_each(|f| walk(&f.ty, out)),
            ast::TypeKind::Fn(a, b) => {
                walk(a, out);
                walk(b, out);
            }
            ast::TypeKind::Var(_) | ast::TypeKind::Unit => {}
        }
    }
    let ast::Decl::Type(t) = d else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut fields: Vec<&ast::FieldType> = t.header.iter().flatten().collect();
    match &t.body {
        ast::TypeBody::New(base, _) => walk(base, &mut out),
        ast::TypeBody::Record(fs) => fields.extend(fs),
        ast::TypeBody::Ctors(cs) => {
            fields.extend(cs.iter().flat_map(|c| c.record.iter().flatten()));
        }
    }
    fields.iter().for_each(|f| walk(&f.ty, &mut out));
    out
}

/// Returns the span of the declaration of the object `r` names in file 0.
fn declared_at(schema: &Schema, r: &Ref) -> Option<Span> {
    let (file, span) = match r {
        Ref::Type(d) | Ref::Ctor(d, _) | Ref::Field { def: d, .. } => {
            let t = schema.ty(*d);
            (t.file, t.span)
        }
        Ref::Relation(rel) => {
            let r = schema.relation(*rel);
            (r.file, r.span)
        }
        Ref::Set(rel, name) => schema
            .relation(*rel)
            .sets
            .iter()
            .find(|s| s.name == *name)?
            .declared_at(),
        Ref::Function(f) => {
            let f = schema.function(*f);
            (f.file, f.span)
        }
        Ref::Aggregate(a) => {
            let a = &schema.aggregates[*a as usize];
            (a.file, a.decl.span)
        }
        Ref::BuiltinType | Ref::Anonymous | Ref::Match(_) | Ref::Unresolved => return None,
    };
    (file == FileId(0)).then_some(span)
}

fn reachable(from: usize, needs: &[Vec<(usize, bool)>]) -> Vec<bool> {
    let mut seen = vec![false; needs.len()];
    let mut stack = vec![from];
    while let Some(i) = stack.pop() {
        for &(j, _) in &needs[i] {
            if !seen[j] {
                seen[j] = true;
                stack.push(j);
            }
        }
    }
    seen
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    New,
    Open,
    Placed,
}

/// Append `i` to `out` after what it needs. Returns the declaration that needs itself, if any.
fn place(
    i: usize,
    needs: &[Vec<usize>],
    state: &mut [Visit],
    out: &mut Vec<usize>,
) -> Result<(), usize> {
    match state[i] {
        Visit::Placed => return Ok(()),
        Visit::Open => return Err(i),
        Visit::New => {}
    }
    state[i] = Visit::Open;
    for &j in &needs[i] {
        place(j, needs, state, out)?;
    }
    state[i] = Visit::Placed;
    out.push(i);
    Ok(())
}
