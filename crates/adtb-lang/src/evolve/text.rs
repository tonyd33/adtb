use super::*;

/// The range of the braces that open at the first character after `at` that is not white space.
pub(super) fn braces_at(text: &str, at: usize) -> Option<Range<usize>> {
    let rest = &text[at..];
    let open = at + rest.find(|c: char| !c.is_whitespace())?;
    if !text[open..].starts_with('{') {
        return None;
    }
    let mut depth = 0;
    for (k, c) in text[open..].char_indices() {
        match c {
            '{' | '(' => depth += 1,
            '}' | ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open..open + k + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// The range of a constructor written at `name`, through its record's braces when it has one.
pub(super) fn ctor_extent(text: &str, name: &ast::Ident, record: bool) -> Range<usize> {
    let start = name.span.start as usize;
    let end = name.span.end as usize;
    if record {
        start..braces_at(text, end).map_or(end, |b| b.end)
    } else {
        start..end
    }
}

/// The end of the comma-separated item that starts at `at`: the first `,`, `}` or `)` outside brackets.
pub(super) fn item_end(text: &str, at: usize) -> usize {
    let mut depth = 0;
    for (k, c) in text[at..].char_indices() {
        match c {
            '{' | '(' => depth += 1,
            '}' | ')' | ',' if depth == 0 => return at + k,
            '}' | ')' => depth -= 1,
            _ => {}
        }
    }
    text.len()
}

/// The edit that adds `item` to the comma-separated items of `braces`.
pub(super) fn add_item(text: &str, braces: Range<usize>, item: Written) -> (Range<usize>, Written) {
    let inner = &text[braces.start + 1..braces.end - 1];
    if inner.trim().is_empty() {
        return (braces, Written::concat(["{ ".into(), item, " }".into()]));
    }
    let last = braces.start + 1 + inner.trim_end().len();
    (last..last, Written::prefixed([item], ", "))
}

/// The range to delete to remove the item starting at `at` from the items of `braces`, and whether it was the only one.
pub(super) fn remove_item(text: &str, braces: Range<usize>, at: usize) -> (Range<usize>, bool) {
    let end = item_end(text, at);
    if text[end..].starts_with(',') {
        let next = end + 1 + text[end + 1..].len() - text[end + 1..].trim_start().len();
        return (at..next, false);
    }
    let before = text[braces.start + 1..at].trim_end();
    if let Some(stripped) = before.strip_suffix(',') {
        let from = braces.start + 1 + stripped.trim_end().len();
        let to = end - (text[at..end].len() - text[at..end].trim_end().len());
        return (from..to, false);
    }
    let from = braces.start - (text[..braces.start].len() - text[..braces.start].trim_end().len());
    (from..braces.end, true)
}

/// The constructors of a type declaration with constructors: name, the range of the name and the end of the constructor, local to `text`.
pub(super) fn ctor_extents(
    text: &str,
    h: usize,
    t: &ast::TypeDecl,
) -> Vec<(String, Range<usize>, usize)> {
    let ast::TypeBody::Ctors(cs) = &t.body else {
        return Vec::new();
    };
    cs.iter()
        .map(|c| {
            let name = c.name.span.start as usize - h..c.name.span.end as usize - h;
            let end = match &c.record {
                Some(_) => braces_at(text, name.end).map_or(name.end, |b| b.end),
                None => name.end,
            };
            (c.name.text.clone(), name, end)
        })
        .collect()
}

/// The range that removing constructor `k` deletes.
pub(super) fn ctor_removal(ctors: &[(String, Range<usize>, usize)], k: usize) -> Range<usize> {
    if k > 0 {
        ctors[k - 1].2..ctors[k].2
    } else if ctors.len() > 1 {
        ctors[0].1.start..ctors[1].1.start
    } else {
        ctors[0].1.start..ctors[0].2
    }
}

/// The braces holding the fields of the header (`ctor` `None`) or of constructor `ctor`, local to `text`.
pub(super) fn field_braces(
    text: &str,
    h: usize,
    t: &ast::TypeDecl,
    ctor: Option<&str>,
) -> Option<Range<usize>> {
    match (&t.body, ctor) {
        (ast::TypeBody::Record(_), _) => {
            let eq = tokens(text)
                .into_iter()
                .find(|(tok, r)| *tok == Tok::Eq && r.start >= t.name.span.end as usize - h)?;
            braces_at(text, eq.1.end)
        }
        (_, None) => {
            t.header.as_ref()?;
            let after = t.params.last().map_or(t.name.span.end, |p| p.span.end) as usize - h;
            braces_at(text, after)
        }
        (ast::TypeBody::Ctors(cs), Some(c)) => {
            let def = cs.iter().find(|d| d.name.text == c)?;
            def.record.as_ref()?;
            braces_at(text, def.name.span.end as usize - h)
        }
        (ast::TypeBody::New(..), _) => None,
    }
}

/// The place to insert the fields of constructor `ctor` when it has none: just after its name.
pub(super) fn ctor_name_end(h: usize, t: &ast::TypeDecl, ctor: &str) -> Option<usize> {
    let ast::TypeBody::Ctors(cs) = &t.body else {
        return None;
    };
    cs.iter()
        .find(|d| d.name.text == ctor)
        .map(|d| d.name.span.end as usize - h)
}

/// The edit adding field text `item` to the header or a constructor of a type declaration.
pub(super) fn add_field_edit(
    text: &str,
    h: usize,
    t: &ast::TypeDecl,
    ctor: Option<&str>,
    item: Written,
) -> Option<(Range<usize>, Written)> {
    if let Some(b) = field_braces(text, h, t, ctor) {
        return Some(add_item(text, b, item));
    }
    let at = match ctor {
        Some(c) => ctor_name_end(h, t, c)?,
        None => t.params.last().map_or(t.name.span.end, |p| p.span.end) as usize - h,
    };
    Some((at..at, Written::concat([" { ".into(), item, " }".into()])))
}

/// The ranges of the clauses of a relation declaration, in order, local to `text`.
pub(super) fn clause_ranges(
    text: &str,
    h: usize,
    r: &ast::RelationDecl,
) -> Option<Vec<Range<usize>>> {
    let toks = tokens(text);
    let after = r.ty.span.end as usize - h;
    let mut depth = 0i32;
    let mut starts = Vec::new();
    let mut prev_interval = false;
    for (tok, range) in &toks {
        if range.start < after {
            continue;
        }
        match tok {
            Tok::LParen | Tok::LBrace | Tok::LBracket => depth += 1,
            Tok::RParen | Tok::RBrace | Tok::RBracket => depth -= 1,
            _ => {}
        }
        let word = &text[range.clone()];
        let start = depth == 0
            && match tok {
                Tok::Kw(crate::lex::Kw::Key) => true,
                Tok::Lower => match word {
                    "index" => !prev_interval,
                    "unique" | "interval" | "default" | "computed" | "volatile" => true,
                    _ => false,
                },
                _ => false,
            };
        prev_interval = *tok == Tok::Lower && word == "interval";
        if start {
            starts.push(range.start);
        }
    }
    if starts.len() != r.clauses.len() {
        return None;
    }
    let mut out = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(text.len());
        out.push(s..s + text[s..end].trim_end().len());
    }
    Some(out)
}

/// The range that removing clause `k` deletes: from the end of what precedes it.
pub(super) fn clause_removal(text: &str, clauses: &[Range<usize>], k: usize) -> Range<usize> {
    let r = clauses[k].clone();
    let from = r.start - (text[..r.start].len() - text[..r.start].trim_end().len());
    from..r.end
}

/// Every expression of a declaration that is checked as a body.
pub(super) fn decl_exprs(d: &Decl) -> Vec<&Expr> {
    match d {
        Decl::Fn(f) => match &f.body {
            ast::FnBody::Expr(e) => vec![e],
            _ => Vec::new(),
        },
        Decl::Constraint(c) => vec![&c.query],
        Decl::Check(c) => vec![&c.predicate],
        Decl::Relation(r) => r
            .clauses
            .iter()
            .flat_map(|c| match c {
                RelationClause::Unique(fs, _) | RelationClause::Index { fields: fs, .. } => {
                    fs.iter().collect()
                }
                RelationClause::Default {
                    value: ast::DefaultValue::Expr(e),
                    ..
                }
                | RelationClause::Computed(_, _, e) => vec![e],
                _ => Vec::new(),
            })
            .collect(),
        Decl::Type(t) => match &t.body {
            ast::TypeBody::New(_, Some(e)) => vec![e],
            _ => Vec::new(),
        },
        Decl::Aggregate(a) => vec![&a.init, &a.step.2, &a.merge.2, &a.finish.1],
        Decl::Set(_) | Decl::Store(_) => Vec::new(),
    }
}

/// The arms of the `match` at `span` in declaration `d`.
pub(super) fn match_arms(d: &Decl, span: Span) -> Option<Vec<ast::Arm>> {
    let mut found = None;
    for e in decl_exprs(d) {
        crate::visit::each_expr(e, &mut |x| {
            if x.span == span
                && let ast::ExprKind::Match(_, arms) = &x.kind
            {
                found = Some(arms.clone());
            }
        });
    }
    found
}

/// Returns the end of the last arm's body of the match at `span` in `d`, inside any parentheses
/// around the match.
pub(super) fn match_end(d: &Decl, span: Span) -> Option<usize> {
    match_arms(d, span)?
        .last()
        .map(|a| a.body.span.end as usize)
}

/// The range of an arm, from the `|` before it to the end of its body, local to `text`.
pub(super) fn arm_range(text: &str, h: usize, a: &ast::Arm) -> Range<usize> {
    let start = a.patterns[0].span.start as usize - h;
    let before = text[..start].trim_end();
    let from = before
        .strip_suffix('|')
        .map_or(start, |b| b.trim_end().len());
    from..a.body.span.end as usize - h
}

/// Returns whether a match over a value of type `ty` covers constructor `c` of `def`.
pub(super) fn holds(schema: &Schema, ty: &Ty, def: TypeId, c: u32) -> bool {
    match ty {
        Ty::Data { def: d, ctors, .. } => {
            *d == def && ctors.as_ref().is_none_or(|cs| cs.contains(&c))
        }
        Ty::Key { rel, ctors } => {
            schema.relation(*rel).elem_def == def && ctors.as_ref().is_none_or(|cs| cs.contains(&c))
        }
        _ => false,
    }
}
