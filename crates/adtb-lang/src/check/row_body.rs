use super::*;

/// A body over one row, as a fill or a routing gives it.
#[derive(Clone, Copy, Debug)]
pub enum RowBody<'e> {
    /// A `match` with no scrutinee, at its span: it matches the row.
    Match(&'e [ast::Arm], Span),
    Expr(&'e ast::Expr),
}

/// Type checks `body` over a row of type `row`, seen from `module`, at `expected` when given.
///
/// A constructor of the row's type that a `match` body builds takes the row's header fields it omits.
/// Returns the schema objects the body names, as [`schema_refs`] reports them.
/// Refuse a body that reads anything but the row: a relation, a reference, a generated key,
/// `caller`, or a function that reads one of them.
/// Refuse, naming the body `writer`, a value it gives or matches where a key type with a late
/// selection is expected.
pub fn check_row(
    schema: &Schema,
    program: &Program,
    module: ModuleId,
    row: &Ty,
    body: RowBody,
    expected: Option<&Ty>,
    writer: &'static str,
) -> Result<Vec<(Span, Ref)>, Diag> {
    let mut c = Checker::of_program(
        schema,
        program,
        Visible::module(module),
        Context::Schema,
        &adhoc::NO_BUNDLES,
        Some(writer),
        BTreeSet::new(),
    );
    row_body(&mut c, program, row, body, expected)?;
    Ok(c.refs.take())
}

/// Type checks `body` as [`check_row`] does, as a query whose one parameter is the row.
pub fn row_query(
    schema: &Schema,
    program: &Program,
    module: ModuleId,
    row: &Ty,
    body: RowBody,
    expected: Option<&Ty>,
) -> Result<Query, Diag> {
    let mut c = Checker::of_program(
        schema,
        program,
        Visible::module(module),
        Context::Schema,
        &adhoc::NO_BUNDLES,
        None,
        BTreeSet::new(),
    );
    let (v, tm, ty) = row_body(&mut c, program, row, body, expected)?;
    let term =
        Build::new(schema, &mut c.terms).lam(v, tm, Ty::Fn(Box::new(row.clone()), Box::new(ty)));
    Ok(Query {
        terms: c.terms,
        term,
    })
}

fn row_body(
    c: &mut Checker<'_>,
    program: &Program,
    row: &Ty,
    body: RowBody,
    expected: Option<&Ty>,
) -> Result<(Var, Tm, Ty), Diag> {
    c.scope.push();
    let v = c.bind(ROW, row.clone(), None, Provenance::Built);
    let (out, span) = match body {
        RowBody::Match(arms, span) => {
            if let Ty::Data { def, .. } = row {
                c.carry = Some((v, *def));
            }
            let scrutinee = ast::Expr {
                kind: ast::ExprKind::Name(ast::QName {
                    module: None,
                    name: ast::Ident {
                        text: ROW.to_string(),
                        span,
                    },
                }),
                span,
            };
            (c.match_expr(&scrutinee, arms, expected, span)?, span)
        }
        RowBody::Expr(e) => {
            let out = match expected {
                Some(t) => c.check(e, t)?,
                None => c.synth(e)?,
            };
            (out, e.span)
        }
    };
    if !reads_only_row(&c.terms, out.tm, program, &mut BTreeSet::new()) {
        return Err(diag(
            span,
            "a fill or a routing reads only its row; this reads a relation, a reference, a generated key, `caller` or a function that does",
        ));
    }
    Ok((v, out.tm, out.ty))
}

/// The name a row body binds its row to, which no source text can spell.
const ROW: &str = "row·";

pub(super) fn reads_only_row(
    terms: &Terms,
    t: Tm,
    program: &Program,
    seen: &mut BTreeSet<FnId>,
) -> bool {
    !terms.any(t, |k| match k {
        Kind::Rel(..)
        | Kind::Transition(..)
        | Kind::Deref { .. }
        | Kind::Fresh(..)
        | Kind::Caller => true,
        Kind::Call { func, .. } => {
            seen.insert(*func)
                && matches!(program.functions.get(func.0 as usize), Some(Some(body))
                    if !reads_only_row(&program.terms, *body, program, seen))
        }
        _ => false,
    })
}
