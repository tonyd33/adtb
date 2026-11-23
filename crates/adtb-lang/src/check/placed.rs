use super::*;
use crate::ast::Expr;

/// A term known to have a type below the slot it was placed for.
#[derive(Clone, Copy, Debug)]
pub(super) struct Placed(Tm);

impl Placed {
    pub(super) fn tm(self) -> Tm {
        self.0
    }
}

impl Checker<'_> {
    /// Returns `out` placed in a slot of type `slot`, refused where its type is not below it.
    pub(super) fn place(&mut self, out: Out, slot: &Ty, span: Span) -> R<Placed> {
        Ok(Placed(self.subsume(out, slot, span)?.tm))
    }

    /// Returns `e` checked against `slot` and placed in it.
    pub(super) fn check_placed(&mut self, e: &Expr, slot: &Ty) -> R<Placed> {
        let o = self.check(e, slot)?;
        self.place(o, slot, e.span)
    }

    /// Returns `tm` placed in a slot of its own type.
    pub(super) fn placed_as_is(&mut self, tm: Tm) -> Placed {
        let ty = self.terms.ty(tm).clone();
        self.placed_checked(tm, &ty)
    }

    /// Returns `tm`, which the checker built for `slot`, placed in it. Record the arena broken at
    /// `tm` when its type is not below `slot`.
    pub(super) fn placed_checked(&mut self, tm: Tm, slot: &Ty) -> Placed {
        let out = self.out(tm);
        match self.place(out, slot, Span::default()) {
            Ok(p) => p,
            Err(d) => {
                self.terms.break_at(tm, d.message);
                Placed(tm)
            }
        }
    }

    /// Returns the set of `x` alone, `x` placed in an element of `set`.
    ///
    /// Preconditions:
    /// - The type of `x` is below the element type of `set`.
    pub(super) fn singleton_checked(&mut self, x: Tm, set: Ty) -> Tm {
        let elem = ty::elem(&set).expect("a set type").clone();
        let x = self.placed_checked(x, &elem);
        self.b().singleton(x, set)
    }

    /// Returns the insert of `rows`, which the checker built as rows of `rel`.
    pub(super) fn insert_checked(&mut self, rel: RelId, rows: Tm) -> Tm {
        let slot = Ty::Set(Box::new(self.schema.relation(rel).elem.clone()));
        let rows = self.placed_checked(rows, &slot);
        self.b().insert(rel, rows)
    }
}

/// A kernel builder whose call argument, constructor and record field, set element, union
/// operand and inserted rows slots take only placed terms.
pub(super) struct SlotBuild<'a>(Build<'a>);

impl<'a> std::ops::Deref for SlotBuild<'a> {
    type Target = Build<'a>;

    fn deref(&self) -> &Build<'a> {
        &self.0
    }
}

impl std::ops::DerefMut for SlotBuild<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'a> SlotBuild<'a> {
    pub(super) fn new(b: Build<'a>) -> SlotBuild<'a> {
        SlotBuild(b)
    }

    pub(super) fn call(&mut self, f: FnId, type_args: Vec<Ty>, args: Vec<Placed>) -> Tm {
        let args = args.into_iter().map(Placed::tm).collect();
        self.0.call(f, type_args, args)
    }

    pub(super) fn ctor(&mut self, c: u32, fields: Vec<(String, Placed)>, ty: Ty) -> Tm {
        let fields = fields.into_iter().map(|(n, p)| (n, p.tm())).collect();
        self.0.ctor(c, fields, ty)
    }

    pub(super) fn record(&mut self, fields: Vec<(String, Placed)>, ty: Ty) -> Tm {
        let fields = fields.into_iter().map(|(n, p)| (n, p.tm())).collect();
        self.0.record(fields, ty)
    }

    pub(super) fn singleton(&mut self, x: Placed, ty: Ty) -> Tm {
        self.0.singleton(x.tm(), ty)
    }

    pub(super) fn app(&mut self, f: Tm, x: Placed) -> Tm {
        self.0.app(f, x.tm())
    }

    pub(super) fn union(&mut self, a: Placed, b: Placed, ty: Ty) -> Tm {
        self.0.union(a.tm(), b.tm(), ty)
    }

    pub(super) fn insert(&mut self, rel: RelId, rows: Placed) -> Tm {
        self.0.insert(rel, rows.tm())
    }
}
