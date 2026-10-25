use crate::ast::*;

/// Calls `f` on `e` and on every expression nested in it, writes and arms included.
pub fn each_expr<'a>(e: &'a Expr, f: &mut dyn FnMut(&'a Expr)) {
    f(e);
    match &e.kind {
        ExprKind::Lit(_) | ExprKind::Name(_) | ExprKind::Unit | ExprKind::Elided => {}
        ExprKind::Call { args, .. } | ExprKind::TypeCall { args, .. } => call_args(args, f),
        ExprKind::Ctor { record, .. } => {
            if let Some(r) = record {
                record_expr(r, f);
            }
        }
        ExprKind::Record(r) => record_expr(r, f),
        ExprKind::Set(es) => es.iter().for_each(|x| each_expr(x, f)),
        ExprKind::Unary(_, x)
        | ExprKind::Reject(x)
        | ExprKind::Field(x, _)
        | ExprKind::Select(x, _)
        | ExprKind::Backlink(x, ..) => each_expr(x, f),
        ExprKind::Binary(_, a, b) => {
            each_expr(a, f);
            each_expr(b, f);
        }
        ExprKind::KeyCall(x, args) => {
            each_expr(x, f);
            match args {
                KeyArgs::Value(v) => each_expr(v, f),
                KeyArgs::Named(ns) => ns.iter().for_each(|(_, v)| each_expr(v, f)),
            }
        }
        ExprKind::Comprehension(c) => comprehension(c, f),
        ExprKind::Match(s, arms) => {
            if let Some(s) = s {
                each_expr(s, f);
            }
            for a in arms {
                if let Some(g) = &a.guard {
                    each_expr(g, f);
                }
                each_expr(&a.body, f);
            }
        }
        ExprKind::Fix(binds, body) => {
            binds.iter().for_each(|(_, b)| each_expr(b, f));
            each_expr(body, f);
        }
        ExprKind::Lambda(_, body) => each_expr(body, f),
        ExprKind::Exists(gens, filter) => {
            gens.iter().for_each(|g| each_expr(&g.source, f));
            if let Some(p) = filter {
                each_expr(p, f);
            }
        }
        ExprKind::Top { count, keys, body } => {
            each_expr(count, f);
            keys.iter().for_each(|k| each_expr(&k.expr, f));
            comprehension(body, f);
        }
        ExprKind::If(c, a, b) => {
            each_expr(c, f);
            each_expr(a, f);
            each_expr(b, f);
        }
        ExprKind::At { point, body, .. } => {
            each_expr(point, f);
            each_expr(body, f);
        }
        ExprKind::Write(w) => each_write_expr(w, f),
    }
}

/// Calls `f` on every expression of a write.
pub fn each_write_expr<'a>(w: &'a Write, f: &mut dyn FnMut(&'a Expr)) {
    match &w.kind {
        WriteKind::Insert { source, upsert, .. } => {
            each_expr(source, f);
            if let Some(u) = upsert {
                if let ConflictTarget::Fields(fs, _) = &u.on {
                    fs.iter().for_each(|x| each_expr(x, f));
                }
                each_write_expr(&u.otherwise, f);
            }
        }
        WriteKind::Update {
            source,
            filter,
            value,
            ..
        } => {
            if let Some(s) = source {
                each_expr(s, f);
            }
            if let Some(p) = filter {
                each_expr(p, f);
            }
            each_expr(value, f);
        }
        WriteKind::Delete { source, filter, .. } => {
            if let Some(s) = source {
                each_expr(s, f);
            }
            if let Some(p) = filter {
                each_expr(p, f);
            }
        }
        WriteKind::Fail(e) => each_expr(e, f),
        WriteKind::Skip => {}
        WriteKind::Seq(ws) => ws.iter().for_each(|w| each_write_expr(w, f)),
        WriteKind::If(c, a, b) => {
            each_expr(c, f);
            each_write_expr(a, f);
            each_write_expr(b, f);
        }
        WriteKind::Match(s, arms) => {
            each_expr(s, f);
            for a in arms {
                if let Some(g) = &a.guard {
                    each_expr(g, f);
                }
                each_write_expr(&a.body, f);
            }
        }
    }
}

/// Calls `f` on every expression of a transact block.
pub fn each_block_expr<'a>(b: &'a TransactBlock, f: &mut dyn FnMut(&'a Expr)) {
    for item in &b.items {
        match item {
            BlockItem::Let(_, LetValue::Expr(e)) => each_expr(e, f),
            BlockItem::Let(_, LetValue::Write(w)) | BlockItem::Write(w) => each_write_expr(w, f),
        }
    }
    if let Some(r) = &b.ret {
        each_expr(r, f);
    }
}

fn call_args<'a>(args: &'a CallArgs, f: &mut dyn FnMut(&'a Expr)) {
    match args {
        CallArgs::List(es) => es.iter().for_each(|x| each_expr(x, f)),
        CallArgs::Aggregate {
            expr,
            generators,
            filter,
        } => {
            generators.iter().for_each(|g| each_expr(&g.source, f));
            if let Some(p) = filter {
                each_expr(p, f);
            }
            each_expr(expr, f);
        }
    }
}

fn record_expr<'a>(r: &'a RecordExpr, f: &mut dyn FnMut(&'a Expr)) {
    match r {
        RecordExpr::Fields(fs, _) => fs.iter().for_each(|(_, x)| each_expr(x, f)),
        RecordExpr::With(base, fs, _) => {
            each_expr(base, f);
            fs.iter().for_each(|(_, x)| each_expr(x, f));
        }
        RecordExpr::Elided(_) => {}
    }
}

fn comprehension<'a>(c: &'a Comprehension, f: &mut dyn FnMut(&'a Expr)) {
    let clauses = |cs: &'a [Clause], f: &mut dyn FnMut(&'a Expr)| {
        for cl in cs {
            match cl {
                Clause::From(gens) => gens.iter().for_each(|g| each_expr(&g.source, f)),
                Clause::Where(p) => each_expr(p, f),
            }
        }
    };
    clauses(&c.clauses, f);
    if let Some((g, after)) = &c.group {
        each_expr(&g.value, f);
        each_expr(&g.by, f);
        clauses(after, f);
    }
    each_expr(&c.select, f);
    if let Some(o) = &c.order {
        o.keys.iter().for_each(|k| each_expr(&k.expr, f));
        if let Some(t) = &o.take {
            each_expr(t, f);
        }
        if let Some(a) = &o.after {
            each_expr(a, f);
        }
    }
}
