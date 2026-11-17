use super::*;
use crate::heap::row::Failed;

#[cfg(test)]
std::thread_local! {
    /// What the calling thread's next [`Heap::refuse_bad`] runs once it has released its latches.
    static BEFORE_REFUSAL: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `hook` once on the calling thread, at its next [`Heap::refuse_bad`], after that releases
/// the caller's latches and before it judges the page.
#[cfg(test)]
pub(in crate::heap) fn before_next_row_refusal(hook: impl FnOnce() + 'static) {
    BEFORE_REFUSAL.with(|b| *b.borrow_mut() = Some(Box::new(hook)));
}

/// Why a stored row does not decode.
pub(in crate::heap) enum Bad {
    /// Bytes on the version's heap page.
    Page,
    /// The pointer in `version`, a version on the heap page, to a value its chunks do not hold.
    Pointer { version: Vec<u8> },
    /// The chunk of an out-of-line value at `at`, holding `tuple`, where its decode stopped.
    Value {
        at: Tid,
        tuple: Vec<u8>,
        cause: Undecoded,
    },
}

impl Heap {
    /// Release `held`, then refuse what `bad` blames: page `pid` as [`Heap::refuse_row`] does, or
    /// the page of the value's chunk. Returns `Ok` when that page changed since, for the
    /// caller to read it again.
    ///
    /// Precondition: `held` is every latch the caller holds.
    pub(in crate::heap) fn refuse_bad<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        held: impl Sized,
        bad: Bad,
    ) -> Result<(), PageError> {
        drop(held);
        #[cfg(test)]
        if let Some(hook) = BEFORE_REFUSAL.with(|b| b.borrow_mut().take()) {
            hook();
        }
        match bad {
            Bad::Page => self.refuse_row(pager, pid, ()),
            Bad::Pointer { version } => self.refuse_pointer(pager, pid, &version),
            Bad::Value { at, tuple, cause } => self.large.refuse_tuple(pager, at, &tuple, cause),
        }
    }

    /// Returns whether `page` is a page of a heap kind this heap owns whose slot table does not
    /// fit it.
    pub(super) fn unframed(&self, page: &impl Owned) -> bool {
        Kinded::of(page).is_some_and(|k| k.owner() == self.id && k.framed().is_none())
    }

    /// Release `held`, then refuse page `pid` as out of frame when, under its exclusive latch,
    /// [`Heap::unframed`] holds of it. Returns `Ok` when it no longer does: the caller reads it
    /// again.
    ///
    /// Precondition: `held` is every latch the caller holds on `pid`.
    pub(super) fn refuse_unframed<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        held: impl Sized,
    ) -> Result<(), PageError> {
        drop(held);
        let judge = |p: &PageExclusive<'_, R>| self.unframed(p).then_some(Shape::Frame);
        pager.refuse_if(pid, judge).map_or(Ok(()), Err)
    }

    /// Release `held`, then refuse page `pid` as [`Shape::Row`] when, under its exclusive latch,
    /// it is a page of the heap holding a version whose bytes on the page do not decode. Returns
    /// `Ok` when the page changed since: the caller reads it again.
    ///
    /// Precondition: `held` is every latch the caller holds on `pid`.
    pub(in crate::heap) fn refuse_row<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        held: impl Sized,
    ) -> Result<(), PageError> {
        drop(held);
        let judge = |p: &PageExclusive<'_, R>| {
            let page = self.page(p)?;
            let format = self.format_of(page)?;
            (0..page.slot_count()).find_map(|slot| {
                let Slot::Normal { at, .. } = page.get(slot) else {
                    return None;
                };
                self.on_page(format, &page[at]).err().map(Shape::Row)
            })
        };
        pager.refuse_if(pid, judge).map_or(Ok(()), Err)
    }

    /// Refuse page `pid` as [`Shape::Row`] of [`Undecoded::Pointer`] when, under its exclusive
    /// latch, it is a page of the heap with a version holding the bytes `version`. Returns `Ok`
    /// when none does: the caller reads it again.
    ///
    /// Precondition: the caller holds no latch on `pid`.
    fn refuse_pointer<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        version: &[u8],
    ) -> Result<(), PageError> {
        let judge = |p: &PageExclusive<'_, R>| {
            let page = self.page(p)?;
            (0..page.slot_count())
                .any(|slot| matches!(page.get(slot), Slot::Normal { at, .. } if page[at.clone()] == *version))
                .then_some(Shape::Row(Undecoded::Pointer))
        };
        pager.refuse_if(pid, judge).map_or(Ok(()), Err)
    }

    /// Fail with why the bytes of `version`, in format `format`, do not decode. Out-of-line
    /// values are not read.
    fn on_page(&self, format: FormatNo, version: &[u8]) -> Result<(), Undecoded> {
        let layout = &self.format(format).layout;
        let body = version.get(HEADER_LEN..).ok_or(Undecoded::Cut)?;
        let (fixed, stored) = layout.split(body)?;
        layout.check(fixed, &stored)
    }

    /// Returns the row of `version` in format `format`, and the pointers of its out-of-line
    /// values, or why the row does not decode.
    pub(in crate::heap) fn decode<R: Runtime>(
        &self,
        pager: &Pager<R>,
        format: FormatNo,
        version: &[u8],
    ) -> Result<Result<Decoded, Bad>, PageError> {
        let layout = &self.format(format).layout;
        let split = version.get(HEADER_LEN..).map(|body| layout.split(body));
        let Some(Ok((fixed, stored))) = split else {
            return Ok(Err(Bad::Page));
        };
        let mut vars = Vec::with_capacity(stored.len());
        let mut pointers = Vec::new();
        for s in &stored {
            match s {
                Stored::Inline(b) => vars.push(b.clone()),
                Stored::Out(p) => {
                    pointers.push(*p);
                    let Ok(value) = self.large.read(pager, *p)? else {
                        let version = version.to_vec();
                        return Ok(Err(Bad::Pointer { version }));
                    };
                    vars.push(value);
                }
            }
        }
        let failed = match layout.decode(fixed, &vars) {
            Ok(row) => return Ok(Ok((row, pointers))),
            Err(failed) => failed,
        };
        Ok(Err(match failed {
            Failed::Var { var, at, cause } => match stored[var] {
                Stored::Out(pointer) => {
                    let found = self.large.holding(pager, pointer, at)?;
                    let (at, tuple) = found.unwrap_or((pointer.first, Vec::new()));
                    Bad::Value { at, tuple, cause }
                }
                Stored::Inline(_) => Bad::Page,
            },
            Failed::Fixed(_) => Bad::Page,
        }))
    }

    pub(super) fn pointers(
        &self,
        format: FormatNo,
        version: &[u8],
    ) -> Result<Vec<Pointer>, Undecoded> {
        let body = version.get(HEADER_LEN..).ok_or(Undecoded::Cut)?;
        let (_, stored) = self.format(format).layout.split(body)?;
        Ok(stored
            .into_iter()
            .filter_map(|s| match s {
                Stored::Out(p) => Some(p),
                Stored::Inline(_) => None,
            })
            .collect())
    }

    /// Returns the versions on `page`, page `pid` of the heap in row format `format`, in slot
    /// order, or why the row of one does not decode.
    pub(in crate::heap) fn versions_on<R: Runtime>(
        &self,
        pager: &Pager<R>,
        pid: Pid,
        page: HeapPage<'_>,
        format: FormatNo,
    ) -> Result<Result<Vec<Version>, Bad>, PageError> {
        let mut found = Vec::new();
        for slot in 0..page.slot_count() {
            let Slot::Normal { flags, at } = page.get(slot) else {
                continue;
            };
            let bytes = &page[at];
            let (row, _) = match self.decode(pager, format, bytes)? {
                Ok(decoded) => decoded,
                Err(bad) => return Ok(Err(bad)),
            };
            found.push(Version {
                tid: tid(pid, slot),
                header: Header::read(bytes),
                flags,
                row: self.upgrade(format, row),
            });
        }
        Ok(Ok(found))
    }

    /// Refuse what [`Heap::refuse_bad`] blames when a version on page `pid` holds a row that does
    /// not decode. A page that is not a page of this heap with sound slots is skipped.
    pub fn check_rows<R: Runtime>(&self, pager: &Pager<R>, pid: Pid) -> Result<(), PageError> {
        loop {
            let guard = pager.read(pid)?;
            let Some(page) = self.page(&guard).filter(|p| p.check().is_empty()) else {
                return Ok(());
            };
            let Some(format) = self.format_of(page) else {
                self.refuse_format(pager, pid, guard)?;
                continue;
            };
            match self.versions_on(pager, pid, page, format)? {
                Ok(_) => return Ok(()),
                Err(bad) => self.refuse_bad(pager, pid, guard, bad)?,
            }
        }
    }
}
