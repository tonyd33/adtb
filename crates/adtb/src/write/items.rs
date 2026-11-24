use adtb_lang::capability::RootKind;
use adtb_lang::kernel::Tm;

/// Normal forms of roots of one kind.
#[derive(Clone, Copy, Debug)]
pub struct Items<'a> {
    items: &'a [Tm],
    kind: RootKind,
}

impl<'a> Items<'a> {
    pub fn new(items: &'a [Tm], kind: RootKind) -> Items<'a> {
        Items { items, kind }
    }

    pub fn kind(self) -> RootKind {
        self.kind
    }
}

impl std::ops::Deref for Items<'_> {
    type Target = [Tm];

    fn deref(&self) -> &[Tm] {
        self.items
    }
}
