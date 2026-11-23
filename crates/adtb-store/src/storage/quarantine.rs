use std::collections::BTreeSet;

use crate::runtime::Runtime;
use crate::steps::{Retire, Step};

use super::alloc::diff;
use super::evidence::{Cause, Evidence, Judged};
use super::order::Ordered;
use super::page::{self, FIRST_EDITABLE, HEADER_LEN, PageKind};
use super::pager::{PageError, PageExclusive, Pager};
use super::policy::{self, Fault, Holder, Mark, Phase};
use super::{Damage, Lsn, Pid, Shape};
use crate::mvcc::Settling;

/// A status page whose file's copy lost a write, as [`Pager::read_stale`] judged it. Dropped
/// unsettled, it refuses the page and records its damage.
#[must_use = "a stale copy dropped unsettled refuses its page"]
pub struct StaleCopy<'a, R: Runtime> {
    pager: &'a Pager<R>,
    lost: Option<PageError>,
}

impl<R: Runtime> std::fmt::Debug for StaleCopy<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaleCopy")
            .field("lost", &self.lost)
            .finish_non_exhaustive()
    }
}

impl<'a, R: Runtime> StaleCopy<'a, R> {
    fn of(pager: &'a Pager<R>, pid: Pid, damage: Damage) -> StaleCopy<'a, R> {
        StaleCopy {
            pager,
            lost: Some(PageError { pid, damage }),
        }
    }

    fn take(&mut self) -> PageError {
        let lost = self.lost.take();
        lost.expect("a stale copy holds its refusal until it is settled or refused")
    }

    /// Refuse the page and record its damage. Returns the refusal.
    ///
    /// Preconditions: as for [`Pager::damage`].
    pub(crate) fn refuse(mut self) -> PageError {
        let e = self.take();
        self.pager.damage(e.pid, e.damage)
    }

    /// Latch the page over `copy`, the file's copy, record it settled, then change it to
    /// `contents`, logging a full image chained from that copy.
    ///
    /// Fail, recording nothing, when the page is refused other than as stale or resident.
    pub(crate) fn settle(
        mut self,
        settling: &Settling,
        copy: &[u8],
        contents: &[u8],
    ) -> Result<(), PageError> {
        let e = self.take();
        let pager = self.pager;
        let mut g = pager.latch_restored(e, copy)?;
        pager.record_settled(settling, e);
        let new = pager.reserve();
        pager.update_all_with(new, &mut [&mut g], |p| {
            p[0][FIRST_EDITABLE..].copy_from_slice(&contents[FIRST_EDITABLE..]);
        })?;
        Ok(())
    }
}

impl<R: Runtime> Drop for StaleCopy<'_, R> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        if let Some(e) = self.lost.take() {
            self.pager.damage(e.pid, e.damage);
        }
    }
}

/// What the allocation bitmap says of a page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Bitmapped {
    Allocated,
    Free,
    /// Its bitmap page is refused.
    Unknown,
}

/// What a detected corruption stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Quarantine {
    /// Every relation: the commit log, or the marks themselves, are damaged.
    Database,
    Heap(u32),
    Index(u32),
}

/// The removal of marks a commit makes: the page holding the marks and the marks it removes.
#[derive(Debug, PartialEq, Eq)]
pub struct Release {
    page: Pid,
    gone: BTreeSet<Quarantine>,
}

impl Release {
    pub fn page(&self) -> Pid {
        self.page
    }
}

const FORMAT: u16 = 1;
const COUNT: usize = HEADER_LEN;
const ENTRIES: usize = HEADER_LEN + 8;
const ENTRY: usize = 8;

impl Quarantine {
    fn encode(self) -> [u8; ENTRY] {
        let (tag, id) = match self {
            Quarantine::Database => (1u32, 0),
            Quarantine::Heap(id) => (2, id),
            Quarantine::Index(id) => (3, id),
        };
        let mut out = [0; ENTRY];
        out[..4].copy_from_slice(&tag.to_le_bytes());
        out[4..].copy_from_slice(&id.to_le_bytes());
        out
    }

    fn decode(bytes: &[u8]) -> Option<Quarantine> {
        let tag = u32::from_le_bytes(bytes[..4].try_into().unwrap());
        let id = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        match tag {
            1 => Some(Quarantine::Database),
            2 => Some(Quarantine::Heap(id)),
            3 => Some(Quarantine::Index(id)),
            _ => None,
        }
    }
}

/// Returns the marks a quarantine page holds, or `None` if it does not decode.
pub fn decode(page: &[u8]) -> Option<BTreeSet<Quarantine>> {
    if page::kind(page) != PageKind::Quarantine as u16 {
        return None;
    }
    let n = u64::from_le_bytes(page[COUNT..COUNT + 8].try_into().unwrap());
    let n = usize::try_from(n).ok()?;
    if n > (page.len() - ENTRIES) / ENTRY {
        return None;
    }
    (0..n)
        .map(|i| Quarantine::decode(&page[ENTRIES + i * ENTRY..ENTRIES + (i + 1) * ENTRY]))
        .collect()
}

fn encode(page: &mut [u8], marks: &BTreeSet<Quarantine>) {
    assert!(
        ENTRIES + marks.len() * ENTRY <= page.len(),
        "{} quarantine marks overflow their page",
        marks.len()
    );
    page[COUNT..].fill(0);
    page[COUNT..COUNT + 8].copy_from_slice(&(marks.len() as u64).to_le_bytes());
    for (i, m) in marks.iter().enumerate() {
        page[ENTRIES + i * ENTRY..ENTRIES + (i + 1) * ENTRY].copy_from_slice(&m.encode());
    }
}

/// Why a walk of an object's pages stopped.
#[derive(Clone, Copy, Debug)]
pub enum Stopped {
    /// On the page named, which failed its checksum or log position.
    Damage(Pid),
    /// On a shape the walk could not follow, which proves nothing: the walk takes no latch, so
    /// the object may have changed under it.
    Shape,
    /// On an I/O failure, which says nothing of the object.
    Io,
}

impl From<PageError> for Stopped {
    fn from(e: PageError) -> Stopped {
        if e.damage == Damage::Unreadable {
            Stopped::Io
        } else {
            Stopped::Damage(e.pid)
        }
    }
}

/// How many times the mapping of a damaged page runs a walk that keeps stopping.
const WALK_TRIES: usize = 16;

/// Returns the outcome of `walk`, repeated while it stops. Damage counts once two walks in a row
/// stop on it, since one walk may have followed a link off a changing object. `Stopped::Shape`
/// when the tries run out.
fn settled(mut walk: impl FnMut() -> Result<Vec<Pid>, Stopped>) -> Result<Vec<Pid>, Stopped> {
    let mut damaged = false;
    for _ in 0..WALK_TRIES {
        match walk() {
            Ok(pages) => return Ok(pages),
            Err(Stopped::Io) => return Err(Stopped::Io),
            Err(Stopped::Damage(at)) if damaged => return Err(Stopped::Damage(at)),
            Err(stop) => damaged = matches!(stop, Stopped::Damage(_)),
        }
    }
    Err(Stopped::Shape)
}

/// What the walks of every object find of one damaged page.
#[derive(Debug)]
pub struct Holders {
    pid: Pid,
    phase: Phase,
    held: BTreeSet<Holder>,
    incomplete: bool,
}

impl Holders {
    /// Take what `walk` finds of `holder`. Leave the mapping incomplete when the walk ends on
    /// anything but damage, or on damage of a holder whose damage marks nothing.
    pub fn walk(&mut self, holder: Holder, walk: impl FnMut() -> Result<Vec<Pid>, Stopped>) {
        let holds = match settled(walk) {
            Ok(pages) => pages.contains(&self.pid),
            Err(Stopped::Damage(_)) => {
                let marks = policy::checksum_mark(holder, self.phase);
                let unmarked = marks.is_nothing();
                self.incomplete |= unmarked;
                !unmarked
            }
            Err(Stopped::Shape | Stopped::Io) => {
                self.incomplete = true;
                false
            }
        };
        if holds {
            self.held.insert(holder);
        }
    }
}

pub(super) use self::marks::{Marks, Unwritable};

/// Why a mark holds only in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkFailure {
    /// The page holding the marks is refused.
    Page(PageError),
    /// The page holding the marks was loaded at this open with no check of its LSN.
    Unverified { page: Pid },
}

impl std::fmt::Display for MarkFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarkFailure::Page(e) => write!(f, "{e}"),
            MarkFailure::Unverified { page } => {
                write!(f, "page {} was loaded with no check of its LSN", page.0)
            }
        }
    }
}

mod marks {
    use std::collections::BTreeSet;
    use std::sync::MutexGuard;
    use std::sync::atomic::{AtomicBool, Ordering};

    use adtb_sys::lock::Lock;

    use super::Quarantine;
    use crate::runtime::Runtime;
    use crate::storage::{PageError, Pid};

    /// The marks in memory, under the marks lock, a leaf, and whether any is set, read without
    /// it.
    ///
    /// Invariants:
    ///
    /// - The flag is set exactly while a mark is set or held; it changes with the marks in one
    ///   hold of the lock.
    /// - The set mirrors the marks page unless the page could not be read at open or was loaded
    ///   unverified.
    #[derive(Debug)]
    pub(in crate::storage) struct Marks {
        state: Lock<State>,
        any: AtomicBool,
        cause: Lock<Option<PageError>>,
        /// Set after `cause` takes damage and cleared after it is emptied, both under its lock;
        /// read without it.
        caused: AtomicBool,
    }

    #[derive(Debug)]
    struct State {
        set: BTreeSet<Quarantine>,
        /// Marks held for this open only, never written.
        held: BTreeSet<Quarantine>,
        page: MarksPage,
    }

    /// The page holding the marks, and whether it takes a write.
    #[derive(Clone, Copy, Debug)]
    enum MarksPage {
        Writable(Pid),
        Unwritable(Unwritable),
    }

    impl MarksPage {
        fn pid(self) -> Pid {
            match self {
                MarksPage::Writable(pid) | MarksPage::Unwritable(Unwritable::Unverified(pid)) => {
                    pid
                }
                MarksPage::Unwritable(Unwritable::Refused(e)) => e.pid,
            }
        }
    }

    /// Why the marks page takes no write.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(in crate::storage) enum Unwritable {
        /// The page was refused at open, with this damage.
        Refused(PageError),
        /// The page was loaded with no check of its LSN against the one expected of it.
        Unverified(Pid),
    }

    impl Marks {
        /// Returns the marks of a file whose header names `page`, none set until they are
        /// loaded.
        pub(in crate::storage) fn new(page: Pid) -> Marks {
            Marks {
                state: Lock::new(State {
                    set: BTreeSet::new(),
                    held: BTreeSet::new(),
                    page: MarksPage::Writable(page),
                }),
                any: AtomicBool::new(false),
                cause: Lock::new(None),
                caused: AtomicBool::new(false),
            }
        }

        /// Keep `e` as the damage of the commit log that refuses the database whole, unless one
        /// is kept already. Returns the one kept.
        pub(in crate::storage) fn refuse_whole<R: Runtime>(
            &self,
            rt: &R,
            e: PageError,
        ) -> PageError {
            let mut cause = self.cause.lock(rt);
            let kept = *cause.get_or_insert(e);
            self.caused.store(true, Ordering::Release);
            kept
        }

        /// Returns the damage of the commit log that refuses the database whole, taking no lock
        /// while there is none.
        pub(in crate::storage) fn cause<R: Runtime>(&self, rt: &R) -> Option<PageError> {
            if !self.caused.load(Ordering::Acquire) {
                return None;
            }
            *self.cause.lock(rt)
        }

        /// Forget the damage of the commit log kept so far.
        pub(in crate::storage) fn forget_cause<R: Runtime>(&self, rt: &R) {
            let mut cause = self.cause.lock(rt);
            *cause = None;
            self.caused.store(false, Ordering::Release);
        }

        fn lock<R: Runtime>(&self, rt: &R) -> Guard<'_> {
            Guard {
                any: &self.any,
                state: self.state.lock(rt),
            }
        }

        /// Take `read`, the marks the page `page` holds, or `Err` with the page's refusal and the
        /// marks its damage sets, which stops every write of the marks. A page read with no
        /// check of its LSN, `checked` false, stops them too.
        pub(super) fn load<R: Runtime>(
            &self,
            rt: &R,
            page: Pid,
            read: Result<BTreeSet<Quarantine>, (PageError, BTreeSet<Quarantine>)>,
            checked: bool,
        ) {
            let mut m = self.lock(rt);
            m.state.page = match (&read, checked) {
                (Err((e, _)), _) => MarksPage::Unwritable(Unwritable::Refused(*e)),
                (Ok(_), false) => MarksPage::Unwritable(Unwritable::Unverified(page)),
                (Ok(_), true) => MarksPage::Writable(page),
            };
            m.state.set = read.unwrap_or_else(|(_, set)| set);
        }

        /// Stop every write of the marks, as a page read with no check of its LSN does.
        pub(in crate::storage) fn unverify<R: Runtime>(&self, rt: &R) {
            let mut m = self.lock(rt);
            if let MarksPage::Writable(page) = m.state.page {
                m.state.page = MarksPage::Unwritable(Unwritable::Unverified(page));
            }
        }

        /// Hold the marks on page `page`, formatted empty.
        pub(in crate::storage) fn formatted<R: Runtime>(&self, rt: &R, page: Pid) {
            self.lock(rt).state.page = MarksPage::Writable(page);
        }

        /// Returns the page holding the marks.
        pub(in crate::storage) fn page<R: Runtime>(&self, rt: &R) -> Pid {
            self.lock(rt).state.page.pid()
        }

        /// Returns the page holding the marks, or why it takes no write.
        pub(in crate::storage) fn writable_page<R: Runtime>(
            &self,
            rt: &R,
        ) -> Result<Pid, Unwritable> {
            match self.lock(rt).state.page {
                MarksPage::Writable(pid) => Ok(pid),
                MarksPage::Unwritable(why) => Err(why),
            }
        }

        /// Returns whether any object is quarantined, without the lock.
        pub(in crate::storage) fn any(&self) -> bool {
            self.any.load(Ordering::Acquire)
        }

        /// Returns every mark, set or held.
        pub(in crate::storage) fn all<R: Runtime>(&self, rt: &R) -> BTreeSet<Quarantine> {
            let m = self.lock(rt);
            m.state.set.union(&m.state.held).copied().collect()
        }

        /// Returns every mark set, without those held for this open only.
        #[cfg(any(test, feature = "testing"))]
        pub(in crate::storage) fn not_held<R: Runtime>(&self, rt: &R) -> BTreeSet<Quarantine> {
            self.lock(rt).state.set.clone()
        }

        /// Returns whether any of `objects` is marked, set or held.
        pub(in crate::storage) fn any_of<R: Runtime>(
            &self,
            rt: &R,
            objects: &[Quarantine],
        ) -> bool {
            let m = self.lock(rt);
            let marked = |o| m.state.set.contains(o) || m.state.held.contains(o);
            objects.iter().any(marked)
        }

        /// Returns the set as `f` would change it, changing nothing.
        pub(in crate::storage) fn changed<R: Runtime>(
            &self,
            rt: &R,
            f: impl FnOnce(&mut BTreeSet<Quarantine>),
        ) -> BTreeSet<Quarantine> {
            let mut set = self.lock(rt).state.set.clone();
            f(&mut set);
            set
        }

        /// Change the set through `f`, in memory only.
        pub(super) fn change<R: Runtime>(&self, rt: &R, f: impl FnOnce(&mut BTreeSet<Quarantine>)) {
            f(&mut self.lock(rt).state.set);
        }

        /// Change the set through `f`. Returns the set, for the caller to write to the marks
        /// page.
        ///
        /// Preconditions: the caller latches the marks page, which [`Marks::writable_page`]
        /// returned.
        pub(super) fn change_on_page<R: Runtime>(
            &self,
            rt: &R,
            f: impl FnOnce(&mut BTreeSet<Quarantine>),
        ) -> BTreeSet<Quarantine> {
            let mut m = self.lock(rt);
            f(&mut m.state.set);
            m.state.set.clone()
        }

        /// Hold `object` marked for this open only.
        pub(super) fn hold<R: Runtime>(&self, rt: &R, object: Quarantine) {
            self.lock(rt).state.held.insert(object);
        }
    }

    /// The marks lock, held. Its drop sets the flag from the marks.
    struct Guard<'a> {
        any: &'a AtomicBool,
        state: MutexGuard<'a, State>,
    }

    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            let any = !self.state.set.is_empty() || !self.state.held.is_empty();
            self.any.store(any, Ordering::Release);
        }
    }
}

impl<R: Runtime> Pager<R> {
    /// Returns a reservation that allocates the page holding the marks, and the page.
    pub(crate) fn reserve_marks(&self) -> Result<(super::Reservation<'_, R>, Pid), PageError> {
        let mut new = self.reserve();
        let pid = new.page(FORMAT, PageKind::Quarantine, 0)?;
        Ok((new, pid))
    }

    /// Read the marks from their page as [`Pager::load_marks`] does.
    #[cfg(any(test, feature = "testing"))]
    pub fn load_marks_tested(&self, page: Pid) -> Result<(), PageError> {
        self.load_marks(page)
    }

    /// Read the marks from their page into memory. A damaged page, or one that does not decode
    /// under its exclusive latch, is refused and quarantines the database.
    ///
    /// Fail, marking nothing, if the page cannot be read.
    pub(crate) fn load_marks(&self, page: Pid) -> Result<(), PageError> {
        let misfit = |p: &PageExclusive<'_, R>| decode(p).is_none().then_some(Shape::Frame);
        let read = loop {
            match self.read(page).map(|p| decode(&p)) {
                Ok(Some(marks)) => break Ok(marks),
                Ok(None) => {
                    if let Some(e) = self.refuse_if(page, misfit) {
                        break Err(e);
                    }
                }
                Err(e) => break Err(e),
            }
        };
        if let Err(e) = read
            && e.damage == Damage::Unreadable
        {
            return Err(e);
        }
        let read = read.map_err(|e| {
            let mark = policy::mark(Fault::Page(e.damage), Holder::MarksPage, Phase::Opening);
            (e, mark.durable().into_iter().collect())
        });
        let checked = self.evidence().proven();
        self.marks().load(self.rt(), page, read, checked);
        Ok(())
    }

    /// Returns every object quarantined.
    pub fn quarantined(&self) -> BTreeSet<Quarantine> {
        self.marks().all(self.rt())
    }

    /// Returns every object marked, without the database held for this open only.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn quarantined_not_held(&self) -> BTreeSet<Quarantine> {
        self.marks().not_held(self.rt())
    }

    /// Returns whether any object is quarantined, without taking a lock.
    pub fn any_quarantined(&self) -> bool {
        self.marks().any()
    }

    /// Returns whether the database is refused whole: by its mark, set or held, or by damage of
    /// the commit log that refuses every transaction.
    pub fn refused_whole(&self) -> bool {
        self.whole_cause().is_some() || self.is_quarantined(Quarantine::Database)
    }

    /// Returns whether this open writes nothing past recovery and its checkpoint: the database is
    /// refused whole, or the read-in gate is unarmed.
    pub fn opens_unwritten(&self) -> bool {
        self.refused_whole() || self.unarmed().is_some()
    }

    /// Returns whether the database is marked, set or held, with no regard to damage of the
    /// commit log.
    pub fn database_marked(&self) -> bool {
        self.is_quarantined(Quarantine::Database)
    }

    /// Returns the damage of the commit log that refuses the database whole, the first kept
    /// this open attempt, taking no lock while there is none.
    pub fn whole_cause(&self) -> Option<PageError> {
        self.marks().cause(self.rt())
    }

    /// Refuse the database whole for `e`, damage of the commit log, keeping the first damage
    /// given since the open attempt began. Returns the damage kept.
    pub(crate) fn refuse_whole(&self, e: PageError) -> PageError {
        self.marks().refuse_whole(self.rt(), e)
    }

    /// Forget the damage of the commit log kept so far.
    ///
    /// Precondition: no transactions layer over this pager serves calls.
    pub fn end_open_attempt(&self, _opening: &super::Opening) {
        self.marks().forget_cause(self.rt());
    }

    /// Returns whether `object` is quarantined, by its own mark or the database's.
    pub fn is_quarantined(&self, object: Quarantine) -> bool {
        self.marks()
            .any_of(self.rt(), &[object, Quarantine::Database])
    }

    /// Mark `object` quarantined. The mark is durable on return, for every caller marking it,
    /// and lasts until [`Pager::release`] removes it. Marking an object already marked and
    /// durable writes nothing.
    ///
    /// Precondition: the calling thread holds no page latch.
    ///
    /// Fail if the page holding the marks is damaged, or was loaded at this open with no check of
    /// its LSN. The mark then holds in memory until the pager closes.
    #[cfg(any(test, feature = "testing"))]
    pub fn quarantine(&self, object: Quarantine, ordered: &Ordered<'_>) -> Result<(), MarkFailure> {
        let mark = BTreeSet::from([Mark::planted(object)]);
        let ((), _, marked) = self.quarantine_judged(ordered, |_| ((), mark));
        marked
    }

    /// Set what each of `causes` that still stands marks on the pages its holders hold during
    /// `phase`: its durable marks in one change of the marks page under one hold of the order
    /// `order` gives, and its marks for an open only held for `opening`. Returns whether each
    /// cause stands, the objects marked durably, and why their marks hold only in memory when
    /// they do.
    ///
    /// Precondition: the calling thread holds no page latch.
    ///
    /// Postcondition: no cause's page changes its damage, incarnation or retirement between the
    /// judgement that it stands and the setting of its marks, durable and held; the durable ones
    /// also have their change of the marks page logged.
    ///
    /// Panic if a cause that stands holds the database for an open only and `opening` is `None`.
    pub(super) fn set_standing(
        &self,
        order: &impl super::Orders,
        phase: Phase,
        causes: Vec<(Cause, BTreeSet<Holder>)>,
        opening: Option<&super::Opening>,
    ) -> (Vec<bool>, BTreeSet<Quarantine>, Result<(), MarkFailure>) {
        let causes: Vec<(Cause, BTreeSet<Mark>)> = causes
            .into_iter()
            .map(|(c, held)| (c, policy::marks(c.fault(), held, phase)))
            .collect();
        let judge = |evidence: &Evidence| {
            let stands: Vec<bool> = causes.iter().map(|(c, _)| evidence.stands(c)).collect();
            let standing = causes.iter().zip(&stands).filter(|(_, s)| **s);
            let set: BTreeSet<Mark> = standing.flat_map(|((_, m), _)| m.iter().copied()).collect();
            if set.iter().any(|m| m.open_only()) {
                let opening = opening.expect("a mark held for an open only is set during an open");
                self.hold_database(opening);
            }
            (stands, set)
        };
        let marks = causes.iter().flat_map(|(_, m)| m);
        match marks.into_iter().any(|m| m.durable().is_some()) {
            true => order.ordered(|ordered| self.quarantine_judged(&ordered, judge)),
            false => (judge(&self.evidence()).0, BTreeSet::new(), Ok(())),
        }
    }

    /// Mark quarantined, as [`Pager::quarantine`] marks each, the objects the marks `judge`
    /// returns from the evidence mark durably. Returns what `judge` returns, the objects marked,
    /// and why their marks hold only in memory when they do.
    ///
    /// Precondition: the calling thread holds no page latch.
    ///
    /// Postcondition: no page's damage or incarnation changes from the start of `judge` until its
    /// marks are set in memory and their change of the marks page is logged.
    fn quarantine_judged<T>(
        &self,
        _: &Ordered<'_>,
        judge: impl FnOnce(&Evidence) -> (T, BTreeSet<Mark>),
    ) -> (T, BTreeSet<Quarantine>, Result<(), MarkFailure>) {
        let durable = |marks: BTreeSet<Mark>| -> BTreeSet<Quarantine> {
            marks.into_iter().filter_map(Mark::durable).collect()
        };
        let latched = self
            .marks_page()
            .and_then(|page| self.exclusive(page).map_err(MarkFailure::Page));
        let mut guard = match latched {
            Ok(guard) => guard,
            Err(failure) => {
                let evidence = self.evidence();
                let (out, marks) = judge(&evidence);
                let objects = durable(marks);
                self.marks()
                    .change(self.rt(), |set| set.extend(objects.iter().copied()));
                return (out, objects, Err(failure));
            }
        };
        let mut append = self.append();
        let (out, marks) = judge(append.evidence());
        let objects = durable(marks);
        let set = self
            .marks()
            .change_on_page(self.rt(), |set| set.extend(objects.iter().copied()));
        let mut page = guard.to_vec();
        encode(&mut page, &set);
        let edits = diff(&guard, &page);
        if !edits.is_empty() {
            append.change_page(&mut guard, edits, false);
        }
        drop(append);
        let lsn = page::lsn(&guard);
        drop(guard);
        self.wait_durable(lsn);
        (out, objects, Ok(()))
    }

    /// Mark `object` quarantined as [`Pager::quarantine`] does, with no fault mapped, in the
    /// commit order `order` gives.
    #[cfg(any(test, feature = "testing"))]
    pub fn plant_mark(
        &self,
        object: Quarantine,
        order: &impl super::Orders,
    ) -> Result<(), MarkFailure> {
        order.ordered(|o| self.quarantine(object, &o))
    }

    /// Hold the database marked for this open only, never written.
    fn hold_database(&self, _opening: &super::Opening) {
        self.marks().hold(self.rt(), Quarantine::Database);
    }

    /// Remove the mark on `object`, once it is repaired. Durable on return.
    #[cfg(any(test, feature = "testing"))]
    pub fn release(&self, object: Quarantine) -> Result<(), MarkFailure> {
        self.change_marks(|set| {
            set.remove(&object);
        })
    }

    /// Returns the exclusive latch on `pid`, over a blank page when it is damaged. The next change
    /// to a blank page logs its whole image, which no earlier record need lead to; dropping the
    /// latch before one refuses the page again.
    ///
    /// Fail on an I/O failure.
    fn latch_overwriting(&self, pid: Pid) -> Result<(PageExclusive<'_, R>, bool), PageError> {
        match self.exclusive(pid) {
            Ok(g) => Ok((g, false)),
            Err(e) if e.pid == pid && e.damage != Damage::Unreadable => {
                Ok((self.latch_overwritten(pid), true))
            }
            Err(e) => Err(e),
        }
    }

    /// Give `pid` the contents of `from` and free `from`, removing the mark on `release` if one is
    /// named and held, in one batch. A damaged `pid` is overwritten unread. Keep the mark on
    /// `release` when the marks page was loaded at this open with no check of its LSN. Returns
    /// once that is durable.
    ///
    /// Preconditions: nothing refers to `from` after the call.
    ///
    /// Fail, changing nothing, if `from` or an allocation bitmap page cannot be read, or the
    /// marks' page when `release` is named.
    pub fn replace_contents(
        &self,
        pid: Pid,
        from: Pid,
        release: Option<Quarantine>,
    ) -> Result<(), PageError> {
        let contents = self.read(from)?.to_vec();
        let page = match release.map(|_| self.marks().writable_page(self.rt())) {
            Some(Ok(page)) => Some(page),
            Some(Err(Unwritable::Refused(e))) => return Err(e),
            Some(Err(Unwritable::Unverified(_))) | None => None,
        };
        let mut marks_page = page.map(|page| self.exclusive(page)).transpose()?;
        let (mut g, blank) = self.latch_overwriting(pid)?;
        let mut new = self.reserve();
        new.free(from);
        let copy = |p: &mut [u8]| {
            p[FIRST_EDITABLE..].copy_from_slice(&contents[FIRST_EDITABLE..]);
        };
        let lsn = match &mut marks_page {
            Some(marks_page) => {
                let unmark = |set: &mut BTreeSet<Quarantine>| {
                    if let Some(release) = &release {
                        set.remove(release);
                    }
                };
                let set = self.marks().changed(self.rt(), unmark);
                let lsn = self.update_all_with(new, &mut [&mut g, marks_page], |p| {
                    copy(p[0]);
                    encode(p[1], &set);
                })?;
                self.marks().change(self.rt(), unmark);
                lsn
            }
            None => self.update_all_with(new, &mut [&mut g], |p| copy(p[0]))?,
        };
        drop(marks_page);
        self.wait_durable(lsn);
        if blank {
            self.write_out(pid, &g);
        }
        Ok(())
    }

    /// Returns page `pid` as the file holds it, with its judgement as stale when the file's copy
    /// lost a write it must have, for the commit log to settle it from the log. A page neither
    /// resident nor refused is judged without refusing it or recording its damage.
    ///
    /// Fail with any other refusal, or when the file's copy is blank, past the log, or fails its
    /// checksum.
    pub(crate) fn read_stale(
        &self,
        pid: Pid,
        _: &Settling,
    ) -> Result<(Vec<u8>, Option<StaleCopy<'_, R>>), PageError> {
        if let Some((copy, damage)) = self.judge_quietly(pid) {
            return Ok((copy, Some(StaleCopy::of(self, pid, damage))));
        }
        let e = match self.read_formatted(pid) {
            Ok(page) => return Ok((page.to_vec(), None)),
            Err(e) => e,
        };
        if !matches!(e.damage, Damage::Stale { .. }) {
            return Err(e);
        }
        match self.read_judged(pid).parts() {
            (Judged::Sound | Judged::Behind(_), Ok(buf)) if page::lsn(&buf) != Lsn(0) => {
                Ok((buf.to_vec(), Some(StaleCopy::of(self, pid, e.damage))))
            }
            _ => Err(e),
        }
    }

    /// Returns the file's copy of `pid` and its damage when the page is neither resident nor
    /// refused, and the copy is valid, allocated and behind its expected LSN with an LSN of its
    /// own, refusing nothing.
    fn judge_quietly(&self, pid: Pid) -> Option<(Vec<u8>, Damage)> {
        let held = self.hold(pid);
        if held.resident() || self.refusal(pid).is_some() {
            return None;
        }
        let (judged, read) = self.read_judged(pid).parts();
        let Judged::Behind(damage @ Damage::Stale { found, .. }) = judged else {
            return None;
        };
        let buf = read.ok()?;
        let lost = found != Lsn(0) && self.bitmapped(pid) != Bitmapped::Free;
        lost.then(|| (buf.to_vec(), damage))
    }

    /// Free damaged page `pid`, overwriting it, so no later read of the file finds the damage.
    ///
    /// Preconditions: nothing refers to `pid`, and it is allocated.
    ///
    /// Fail, leaving `pid` as it was, if an allocation bitmap page cannot be read.
    pub fn scrap(&self, pid: Pid) -> Result<(), PageError> {
        let (mut g, _) = self.latch_overwriting(pid)?;
        let mut new = self.reserve();
        new.free(pid);
        self.update_all_with(new, &mut [&mut g], |p| p[0][HEADER_LEN] = 1)?;
        self.write_out(pid, &g);
        Ok(())
    }

    /// Returns the page holding the marks.
    pub fn quarantine_page(&self) -> Pid {
        self.marks().page(self.rt())
    }

    #[cfg(any(test, feature = "testing"))]
    fn change_marks(&self, f: impl FnOnce(&mut BTreeSet<Quarantine>)) -> Result<(), MarkFailure> {
        let page = self.marks_page()?;
        self.change_marks_on(page, f).map_err(MarkFailure::Page)
    }

    /// Returns the page holding the marks, or why a mark holds only in memory.
    fn marks_page(&self) -> Result<Pid, MarkFailure> {
        match self.marks().writable_page(self.rt()) {
            Ok(page) => Ok(page),
            Err(Unwritable::Refused(e)) => Err(MarkFailure::Page(e)),
            Err(Unwritable::Unverified(page)) => Err(MarkFailure::Unverified { page }),
        }
    }

    /// Returns the removal of the marks of `gone` that are set, for a commit to make in its batch,
    /// `None` when none is set.
    ///
    /// Fail with the first mark of `gone` that is set and why the page holding the marks takes
    /// no write: it is refused, or was loaded at this open with no check of its LSN.
    pub fn releasing(
        &self,
        gone: &BTreeSet<Quarantine>,
    ) -> Result<Option<Release>, (Quarantine, MarkFailure)> {
        let gone: BTreeSet<Quarantine> = self
            .quarantined()
            .intersection(gone)
            .filter(|o| **o != Quarantine::Database)
            .copied()
            .collect();
        let Some(&first) = gone.first() else {
            return Ok(None);
        };
        let page = self.marks_page().map_err(|why| (first, why))?;
        Ok(Some(Release { page, gone }))
    }

    /// Remove the marks of `r` in memory and write the marks left to `page`, the contents of the
    /// page of `r` a commit's batch changes.
    ///
    /// Preconditions:
    /// - The caller latches the page of `r` exclusively and logs the change in the batch of a
    ///   commit that drops each structure `r` names or builds it again whole.
    pub fn release_on(&self, r: &Release, page: &mut [u8]) {
        let set = self
            .marks()
            .change_on_page(self.rt(), |set| set.retain(|o| !r.gone.contains(o)));
        encode(page, &set);
    }

    /// Set the marks of `r` again in memory, after the commit that was to remove them failed.
    pub fn unrelease(&self, r: &Release) {
        self.marks()
            .change_on_page(self.rt(), |set| set.extend(r.gone.iter().copied()));
    }

    /// Remove the mark on each heap and index `gone` accepts. Durable on return.
    ///
    /// Precondition: each heap or index `gone` accepts is dropped, or built again whole since its
    /// mark was set, and no structure but that new build has its id.
    ///
    /// Fail, removing nothing, if the page holding the marks is refused, or was loaded at this open
    /// with no check of its LSN.
    pub fn release_gone(&self, gone: impl Fn(Quarantine) -> bool) -> Result<(), MarkFailure> {
        let gone = |object: &Quarantine| *object != Quarantine::Database && gone(*object);
        if !self.any_quarantined() || !self.quarantined().iter().any(gone) {
            return Ok(());
        }
        let page = self.marks_page()?;
        self.change_marks_on(page, |set| set.retain(|object| !gone(object)))
            .map_err(MarkFailure::Page)
    }

    /// Change the marks through `f` and write them to their page `page`, durable on return.
    ///
    /// Fail, changing nothing, if the page is refused.
    fn change_marks_on(
        &self,
        page: Pid,
        f: impl FnOnce(&mut BTreeSet<Quarantine>),
    ) -> Result<(), PageError> {
        let mut guard = self.exclusive(page)?;
        let set = self.marks().change_on_page(self.rt(), f);
        guard.update(|p| encode(p, &set));
        let lsn = page::lsn(&guard);
        drop(guard);
        self.wait_durable(lsn);
        Ok(())
    }

    /// Returns what holds the damaged page `pid` during `phase`, `None` when the mapping is
    /// incomplete and found no holder. `walk` reports the pages of every object to the
    /// [`Holders`].
    ///
    /// Postconditions:
    ///
    /// - A page the allocation bitmap calls free, before or after the walk, is held as free
    ///   unless a walk found it. A page whose bitmap page is refused counts as allocated.
    /// - A page held as retired, before or after the walk, is held as retired.
    /// - A walk stopped by damage holds the page, or leaves the mapping incomplete when its
    ///   holder's damage marks nothing. One stopped by an I/O failure, or on a shape it could not
    ///   follow in all its tries, leaves the mapping incomplete.
    /// - An allocation bitmap page, the damage list's page, the marks page and a page of the page
    ///   LSN map are held as such, with no walk. A map page below a refused one is not found as
    ///   one.
    /// - A page no walk holds is unreached, once every walk is complete.
    #[cfg(any(test, feature = "testing"))]
    pub fn holders_of(
        &self,
        pid: Pid,
        phase: Phase,
        walk: impl FnOnce(&mut Holders),
    ) -> Option<BTreeSet<Holder>> {
        self.holders_meeting(pid, phase, walk)
    }

    /// Returns what [`Pager::holders_of`] does.
    pub fn holders_meeting(
        &self,
        pid: Pid,
        phase: Phase,
        walk: impl FnOnce(&mut Holders),
    ) -> Option<BTreeSet<Holder>> {
        match self.held_unwalked(pid) {
            Some(holder) => Some(BTreeSet::from([holder])),
            None => self.holders_walked(pid, phase, walk),
        }
    }

    /// Returns what holds `pid` with no walk: an allocation bitmap page, the damage list's page,
    /// the marks page or a page of the page LSN map.
    pub fn held_unwalked(&self, pid: Pid) -> Option<Holder> {
        if self.is_bitmap_page(pid) {
            return Some(Holder::Bitmap);
        }
        if pid != Pid(0) && pid == self.damage_list_page() {
            return Some(Holder::DamageList);
        }
        if pid != Pid(0) && pid == self.quarantine_page() {
            return Some(Holder::MarksPage);
        }
        if self.holds_map_page(pid) {
            return Some(Holder::LsnMap);
        }
        None
    }

    /// Returns what holds the damaged page `pid` during `phase` from the walks `walk` reports
    /// alone and the pages held as retired, `None` when the mapping is incomplete and found no
    /// holder. No allocation bitmap, damage list, marks or page LSN map page is held as such: a
    /// page no walk holds is free or unreached by the allocation bitmap.
    pub fn holders_walked(
        &self,
        pid: Pid,
        phase: Phase,
        walk: impl FnOnce(&mut Holders),
    ) -> Option<BTreeSet<Holder>> {
        self.evidence().walked_one();
        let before = self.bitmapped(pid);
        let retired_before = self.retired().contains(&pid);
        let mut h = Holders {
            pid,
            phase,
            held: BTreeSet::new(),
            incomplete: false,
        };
        walk(&mut h);
        if retired_before || self.retired().contains(&pid) {
            h.held.insert(Holder::Retired);
        }
        if !h.held.is_empty() {
            return Some(h.held);
        }
        if h.incomplete {
            return None;
        }
        if before == Bitmapped::Free || self.bitmapped(pid) == Bitmapped::Free {
            return Some(BTreeSet::from([Holder::Free]));
        }
        Some(BTreeSet::from([Holder::Unreached]))
    }

    /// Hold `pids`, pages of a tree a swap replaced, as retired until each one's own free.
    pub fn retire(&self, pids: impl IntoIterator<Item = Pid>) {
        self.evidence().retire(pids);
        self.steps().at(Step::Retire(Retire::Held));
    }

    /// Stop holding `pids` as retired: the swap that would have replaced their tree failed.
    pub fn unretire(&self, pids: &[Pid]) {
        self.evidence().unretire(pids);
    }

    /// Returns the pages held as retired, none of them freed since.
    pub fn retired(&self) -> Vec<Pid> {
        self.evidence().retired()
    }

    /// Returns what the allocation bitmap says of `pid`.
    pub(super) fn bitmapped(&self, pid: Pid) -> Bitmapped {
        match self.is_free(pid) {
            Ok(true) => Bitmapped::Free,
            Ok(false) => Bitmapped::Allocated,
            Err(_) => Bitmapped::Unknown,
        }
    }

    /// Format the marks page `pid`, reserved by `new`, empty.
    pub(crate) fn format_marks(
        &self,
        new: super::Reservation<'_, R>,
        pid: Pid,
    ) -> Result<(), PageError> {
        self.update_with(new, &[pid], |p| encode(p[0], &BTreeSet::new()))?;
        self.marks().formatted(self.rt(), pid);
        Ok(())
    }
}

#[cfg(test)]
#[path = "marks_tests.rs"]
mod tests;
