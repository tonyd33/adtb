use crate::plan::{Catalog, Chain, End, Eval, Pipeline, Reads, Sink, Source, Spine, Step};

pub(super) fn collect_eval(c: &Catalog, e: &Eval, out: &mut Reads) {
    match e {
        Eval::Expr(_) | Eval::GroupRows { .. } => {}
        Eval::Let(_, a, b) | Eval::If(_, a, b) => {
            collect_eval(c, a, out);
            collect_eval(c, b, out);
        }
        Eval::Case(_, arms) => arms.iter().for_each(|(_, _, a)| collect_eval(c, a, out)),
        Eval::Try(a) => collect_eval(c, a, out),
        Eval::At(a) => collect_eval(c, &a.body, out),
        Eval::Lookup(l) => {
            out.indexes.insert(l.index);
            out.heaps.extend(l.routes.iter().map(|(_, h)| *h));
        }
        Eval::Query(q) => {
            for b in &q.builds {
                b.pipelines.iter().for_each(|p| collect_pipeline(c, p, out));
            }
            collect_spine(c, &q.spine, out);
            match &q.sink {
                Sink::Set | Sink::Exists | Sink::IsEmpty => {}
                Sink::Agg { body, .. } | Sink::Fold { body, .. } => collect_eval(c, body, out),
                Sink::Group { by, aggs, .. } => {
                    collect_eval(c, by, out);
                    for a in aggs {
                        a.filter.iter().for_each(|f| collect_eval(c, f, out));
                        collect_eval(c, &a.element, out);
                        collect_eval(c, &a.body, out);
                    }
                }
                Sink::Top { count, keys, .. } => {
                    collect_eval(c, count, out);
                    keys.iter().for_each(|(k, _)| collect_eval(c, k, out));
                }
                Sink::Present {
                    keys, take, after, ..
                } => {
                    keys.iter().for_each(|(k, _)| collect_eval(c, k, out));
                    take.iter()
                        .chain(after)
                        .for_each(|x| collect_eval(c, x, out));
                }
            }
        }
        Eval::Fix(f) => {
            for b in &f.binds {
                collect_eval(c, &b.body, out);
                if let Some(s) = &b.step {
                    collect_eval(c, s, out);
                }
            }
            collect_eval(c, &f.body, out);
        }
    }
}

fn collect_spine(c: &Catalog, s: &Spine, out: &mut Reads) {
    match s {
        Spine::Pipeline(p) => collect_pipeline(c, p, out),
        Spine::Union(ss) => ss.iter().for_each(|s| collect_spine(c, s, out)),
        Spine::If(_, a, b) => {
            collect_spine(c, a, out);
            collect_spine(c, b, out);
        }
        Spine::Case(_, arms) => arms.iter().for_each(|(_, _, s)| collect_spine(c, s, out)),
        Spine::Let(_, e, b) => {
            collect_eval(c, e, out);
            collect_spine(c, b, out);
        }
    }
}

fn collect_pipeline(c: &Catalog, p: &Pipeline, out: &mut Reads) {
    if let Source::Scan { heap, .. } = p.source {
        out.heaps.insert(heap);
    }
    collect_chain(c, &p.chain, out);
}

fn collect_chain(c: &Catalog, ch: &Chain, out: &mut Reads) {
    for s in &ch.steps {
        if let Step::Bind(_, e) = s {
            collect_eval(c, e, out);
        }
    }
    match &ch.end {
        End::Nothing | End::Emit(_) => {}
        End::Fork(cs) => cs.iter().for_each(|x| collect_chain(c, x, out)),
        End::If(_, a, b) => {
            collect_chain(c, a, out);
            collect_chain(c, b, out);
        }
        End::Case(_, arms) => arms.iter().for_each(|(_, _, x)| collect_chain(c, x, out)),
        End::Index { index, routes, .. } | End::Range { index, routes, .. } => {
            out.indexes.insert(*index);
            for r in routes {
                out.heaps.insert(r.heap);
                collect_chain(c, &r.chain, out);
            }
        }
    }
}
