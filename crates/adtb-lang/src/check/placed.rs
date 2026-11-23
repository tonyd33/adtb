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
        self.singleton_placed(x, set)
    }

    pub(super) fn call_placed(&mut self, f: FnId, type_args: Vec<Ty>, args: Vec<Placed>) -> Tm {
        let args = args.into_iter().map(Placed::tm).collect();
        self.b().call(f, type_args, args)
    }

    pub(super) fn ctor_placed(&mut self, c: u32, fields: Vec<(String, Placed)>, ty: Ty) -> Tm {
        let fields = fields.into_iter().map(|(n, p)| (n, p.tm())).collect();
        self.b().ctor(c, fields, ty)
    }

    pub(super) fn singleton_placed(&mut self, x: Placed, ty: Ty) -> Tm {
        self.b().singleton(x.tm(), ty)
    }

    pub(super) fn app_placed(&mut self, f: Tm, x: Placed) -> Tm {
        self.b().app(f, x.tm())
    }
}
