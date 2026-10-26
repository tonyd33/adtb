use super::*;
use crate::canon;

impl Source {
    /// Returns the names of each declaration, by span in its file of [`Source::files`], declaration `i`
    /// at index `i`.
    pub fn names(&self) -> Result<Vec<Names>, Vec<FileDiag>> {
        let schema = self.schema()?;
        let mut out = vec![Names::default(); self.decls.len()];
        for (file, f) in self.files()? {
            let module = schema
                .module_named(&f.module.text)
                .unwrap_or(schema::PRELUDE);
            for d in &f.decls {
                canon::decl_names(&schema, module, d, &mut out[file.0 as usize]);
            }
        }
        for (file, span, r) in check::schema_refs(&schema) {
            if let Some(names) = out.get_mut(file.0 as usize) {
                names.note(&schema, span, &r);
            }
        }
        Ok(out)
    }
}

impl Logged {
    /// Returns the names of this migration's text, by span.
    ///
    /// An operand the operator writes into a declaration (a declaration, a constructor, a field type, an
    /// arm, an index field, a default, a computed field) resolves there, in the source after the
    /// operator, and when that source does not build, in the migration's result. The fields of a
    /// dropped unique or index resolve in the clause before the operator. A fill or a routing resolves
    /// as the operator checks it.
    pub fn names(&self) -> Result<Names, Vec<Diag>> {
        let mut src = (*self.before).clone();
        src.version += 1;
        let mut names = Names::default();
        let mut late = Vec::new();
        for op in &self.operators {
            let prior = src.clone();
            let noted = RefCell::new(Names::default());
            src.apply_noting(&self.module, op, &self.text, Some(&noted))?;
            names.extend(noted.into_inner());
            let (written, dropped) = copies(op, &self.text);
            late.extend(resolve(&mut names, &self.text, written, &src, &prior));
            let unresolved = resolve(&mut names, &self.text, dropped, &prior, &src);
            if let Some(r) = unresolved.first() {
                return Err(vec![unlocated(r)]);
            }
        }
        if late.is_empty() {
            return Ok(names);
        }
        let unbuilt = |ds: Vec<FileDiag>| ds.into_iter().map(|d| d.diag).collect::<Vec<_>>();
        let result = src.names().map_err(unbuilt)?;
        let h = |i: usize| header(&src.decls[i].module).len();
        let mut missing = Vec::new();
        for r in late {
            let slice = &self.text[r.clone()];
            let found: Vec<(usize, usize)> = (0..src.decls.len())
                .flat_map(|i| {
                    src.decls[i]
                        .text
                        .match_indices(slice)
                        .map(move |(o, _)| (i, o))
                })
                .collect();
            match found.as_slice() {
                [(i, o)] => names.copy(&result[*i], h(*i) + o..h(*i) + o + r.len(), r.start),
                _ => missing.push(unlocated(&r)),
            }
        }
        if missing.is_empty() {
            Ok(names)
        } else {
            Err(missing)
        }
    }
}

fn unlocated(r: &Range<usize>) -> Diag {
    Diag::new(
        Span::new(r.start, r.end),
        "no declaration holds this operand's text, so its names do not resolve",
    )
}

/// Copies into `names` the names of each range of `text` from the declaration of `changed` that holds
/// it where `changed` differs from `other`. Returns the ranges it finds no such declaration for, or
/// all of them when `changed` does not build.
fn resolve(
    names: &mut Names,
    text: &str,
    ranges: Vec<Range<usize>>,
    changed: &Source,
    other: &Source,
) -> Vec<Range<usize>> {
    if ranges.is_empty() {
        return ranges;
    }
    let Ok(decl_names) = changed.names() else {
        return ranges;
    };
    let mut missing = Vec::new();
    for r in ranges {
        match locate(changed, other, &text[r.clone()]) {
            Some((i, at)) => {
                let h = header(&changed.decls[i].module).len();
                names.copy(&decl_names[i], h + at..h + at + r.len(), r.start);
            }
            None => missing.push(r),
        }
    }
    missing
}

/// Returns the declaration of `changed` and the offset in its text of an occurrence of `slice` that
/// overlaps what differs from the same declaration of `other`; a declaration `other` lacks differs whole.
fn locate(changed: &Source, other: &Source, slice: &str) -> Option<(usize, usize)> {
    changed.decls.iter().enumerate().find_map(|(i, d)| {
        let region = match other.decls.get(i) {
            Some(o) if o.text == d.text => return None,
            Some(o) => differing(&d.text, &o.text),
            None => 0..d.text.len(),
        };
        d.text
            .match_indices(slice)
            .map(|(at, _)| at)
            .find(|&at| at < region.end && at + slice.len() > region.start)
            .map(|at| (i, at))
    })
}

/// The range of `a` that differs from `b`, between their common prefix and common suffix.
fn differing(a: &str, b: &str) -> Range<usize> {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let room = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    prefix..a.len() - suffix
}

fn arm_range(a: &ast::ArmClause) -> Range<usize> {
    a.pattern.span.start as usize..a.body.span.end as usize
}

/// The ranges of `op`'s text it writes into declarations, and the ranges naming what it removes from one.
fn copies(op: &Operator, text: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let ctor = |d: &ast::CtorDef| ctor_extent(text, &d.name, d.record.is_some());
    let exprs = |es: &[Expr]| es.iter().map(|e| e.span.range()).collect::<Vec<_>>();
    let arms = |cs: &[ast::ArmClause]| cs.iter().map(arm_range).collect::<Vec<_>>();
    let written = match &op.kind {
        OperatorKind::AddType(t) => vec![t.span.range()],
        OperatorKind::AddRelation(r) => vec![r.span.range()],
        OperatorKind::AddSet(s) => vec![s.span.range()],
        OperatorKind::DefineFn(f) | OperatorKind::RedefineFn(f) => vec![f.span.range()],
        OperatorKind::AddConstraint(c) => vec![c.span.range()],
        OperatorKind::AddCheck(c) => vec![c.span.range()],
        OperatorKind::Store(s) => vec![s.span.range()],
        OperatorKind::DefineAggregate(a) => vec![a.span.range()],
        OperatorKind::AddConstructor {
            path,
            record,
            arms: a,
            ..
        } => {
            let mut out = vec![ctor_extent(text, &path.ctor, record.is_some())];
            out.extend(arms(a));
            out
        }
        OperatorKind::SplitConstructor { into, clauses, .. } => {
            let mut out: Vec<Range<usize>> = into.iter().map(ctor).collect();
            for c in clauses {
                match c {
                    ast::SplitClause::Arm(a) => out.push(arm_range(a)),
                    ast::SplitClause::Unique(fs, _)
                    | ast::SplitClause::Index { fields: fs, .. } => out.extend(exprs(fs)),
                }
            }
            out
        }
        OperatorKind::MergeConstructor { into, clauses, .. }
        | OperatorKind::NestConstructor { into, clauses, .. } => {
            let mut out = vec![ctor(into)];
            out.extend(arms(clauses));
            out
        }
        OperatorKind::UnnestConstructor(_, clauses) => arms(clauses),
        OperatorKind::AddField { ty, .. } => vec![ty.span.range()],
        OperatorKind::AddUnique(fs, _) | OperatorKind::AddIndex { fields: fs, .. } => exprs(fs),
        OperatorKind::AddDefault {
            value: ast::DefaultValue::Expr(e),
            ..
        } => vec![e.span.range()],
        OperatorKind::AddComputed { ty, expr, .. }
        | OperatorKind::RedefineComputed { ty, expr, .. } => {
            vec![ty.span.range(), expr.span.range()]
        }
        _ => Vec::new(),
    };
    let dropped = match &op.kind {
        OperatorKind::DropUnique(fs, _) | OperatorKind::DropIndex { fields: fs, .. } => exprs(fs),
        _ => Vec::new(),
    };
    (written, dropped)
}
