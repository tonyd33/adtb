//! A continuous archive of the log: each complete segment copied to a destination directory under
//! the file's id and epoch, named by its first LSN and the blake3 hash of its bytes, in order and
//! gapless. The tail past the last complete segment is never archived, so a restore from the
//! archive reaches at most the end of the last complete segment.
//!
//! Lock order: at open, the marks lock, then the meta lock, then the header lock, each brief and
//! never nested in each other; before the first pass, the segments lock alone. In
//! a pass, the pass lock, then the log lock or the status lock, each brief and never nested in
//! each other. A pass never takes the segments lock: a complete segment is never written again.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use adtb_sys::lock::Lock;

use crate::runtime::{Done, FileId, Io as _, IoError, Op, Runtime, ThreadId};

use super::Lsn;
use super::backup::{BackupError, Fs, IO_CHUNK, Io, PARTIAL, inside, io_error};
use super::digest::hex;
use super::header::REGION;
use super::log::{parse_segment, segment_name};
use super::pager::{Pager, parent};
use super::quarantine::Quarantine;

const EPOCH: &str = "epoch";
const EVERY: Duration = Duration::from_secs(1);

/// What the archive holds, as the last pass found it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The end of the gapless run of published segments of this epoch from its first segment,
    /// within the log this file holds; `None` until a pass has listed the destination, and when
    /// the run does not start at the first segment.
    pub through: Option<Lsn>,
    /// Why the last pass stopped, `None` when it finished. A segment missing from the log stops
    /// every later pass there; only a new destination gets past it.
    pub failure: Option<BackupError>,
}

#[derive(Debug, Default)]
struct Control {
    stop: bool,
    thread: Option<ThreadId>,
}

#[derive(Debug)]
/// The archive of a pager's log.
pub struct Archiver<R: Runtime> {
    pager: Pager<R>,
    /// The directory that must exist for the archive to be written.
    dest: String,
    /// The directory of this file's id and epoch.
    dir: String,
    /// The base of the segment holding the header's `log_start`.
    first: u64,
    segment: u64,
    /// The epoch file's bytes.
    epoch: Vec<u8>,
    /// Held across a whole pass, with the LSN below which published segments match the log.
    pass: Lock<u64>,
    status: Lock<Status>,
    control: Lock<Control>,
}

impl<R: Runtime> Archiver<R> {
    /// Returns an archiver of `pager`'s log into `dest`, when given, and bound what `pager`'s
    /// recycle removes by what it has archived.
    pub fn bounding(pager: &Pager<R>, dest: Option<String>) -> Option<Arc<Archiver<R>>> {
        let archiver = Arc::new(Archiver::new(pager.clone(), &dest?));
        let weak = Arc::downgrade(&archiver);
        pager.set_archive(Arc::new(move || {
            weak.upgrade().and_then(|a| a.archived_through())
        }));
        Some(archiver)
    }

    /// Returns an archiver of `pager`'s log into `dest`.
    pub fn new(pager: Pager<R>, dest: &str) -> Archiver<R> {
        let (id, epoch, segment, start, branch) = pager.read_header(|h| {
            let branch = h.history.last().copied().filter(|_| h.log_start.0 > 0);
            (
                hex(&h.file_id),
                h.epoch,
                h.segment_size,
                h.log_start.0,
                branch,
            )
        });
        let mut text = match branch {
            Some((epoch, end)) => format!("adtb archive epoch\nparent {epoch} {}\n", end.0),
            None => "adtb archive epoch\nparent none\n".to_owned(),
        }
        .into_bytes();
        text.resize(text.len().next_multiple_of(REGION), 0);
        Archiver {
            segment,
            epoch: text,
            dest: dest.to_owned(),
            dir: inside(&inside(dest, &id), &format!("{epoch:016x}")),
            first: start - start % segment,
            pager,
            pass: Lock::new(0),
            status: Lock::new(Status::default()),
            control: Lock::new(Control::default()),
        }
    }

    pub fn status(&self) -> Status {
        self.status.lock(self.pager.rt()).clone()
    }

    /// Returns the LSN below which every segment of this epoch is published at the destination.
    /// A recycler keeps every segment while it is `None` or below the segment's end.
    pub fn archived_through(&self) -> Option<Lsn> {
        self.status().through
    }

    /// Start a thread that runs a pass every second until [`stop`](Self::stop).
    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        let thread = self.pager.rt().spawn_daemon("archive", move || me.run());
        self.control.lock(self.pager.rt()).thread = Some(thread);
    }

    /// Stop the thread and wait for it to end.
    pub fn stop(&self) {
        let thread = {
            let mut c = self.control.lock(self.pager.rt());
            c.stop = true;
            c.thread.take()
        };
        if let Some(t) = thread {
            self.pager.rt().unpark(t);
            self.pager.rt().join(t);
        }
    }

    fn run(&self) {
        let rt = self.pager.rt();
        let mut io = rt.io();
        let mut timer = false;
        while !self.control.lock(rt).stop {
            self.archive();
            if !timer {
                io.submit(Op::Timer {
                    at: rt.now() + EVERY,
                });
                timer = true;
            }
            if !self.control.lock(rt).stop {
                rt.park();
            }
            while io.poll().is_some() {
                timer = false;
            }
        }
    }

    /// Run a pass and record its outcome in the status.
    pub fn archive(&self) {
        let log_dir = self.pager.segments().lock(self.pager.rt()).dir.clone();
        let failure = self.pass(&log_dir).err();
        self.status.lock(self.pager.rt()).failure = failure;
    }

    /// Publish every complete segment of the log not yet at the destination, in order.
    ///
    /// Postconditions:
    /// - The epoch file is published before any segment, and a segment only after the one before
    ///   it, unless it is the directory's first.
    /// - A published name is never replaced.
    /// - `through` moves only past a segment whose name a forced directory holds.
    fn pass(&self, log_dir: &str) -> Io<()> {
        let rt = self.pager.rt();
        let mut checked = self.pass.lock(rt);
        let mut fs = Fs::new(rt.clone());
        for dir in [parent(&self.dir), self.dir.as_str()] {
            match fs
                .drv
                .attempt(Op::CreateDir {
                    dir: dir.to_owned(),
                })
                .result
            {
                Ok(Done::Created) => fs.sync_dir(parent(dir))?,
                Ok(_) => {}
                Err(IoError::NotFound) => return Err(io_error(&self.dest)(IoError::NotFound)),
                Err(e) => return Err(io_error(dir)(e)),
            }
        }
        let (published, epoch) = self.listed(&mut fs)?;
        self.publish_epoch(&mut fs, epoch)?;
        let (segment, durable) = (self.segment, self.pager.durable().0);
        let mut next = self.first;
        while let Some(held) = published.get(&next).filter(|_| next + segment <= durable) {
            if next >= *checked {
                let path = inside(log_dir, &segment_name(next));
                if read_segment(&mut fs, &path, None)?.is_some_and(|h| hex(h.as_bytes()) != *held) {
                    self.status.lock(rt).through = Some(Lsn(next));
                    return Err(BackupError::Damaged {
                        part: format!("{}-{held}", segment_name(next)),
                    });
                }
                *checked = next + segment;
            }
            next += segment;
        }
        let starts = published.is_empty() || published.contains_key(&self.first);
        self.status.lock(rt).through = starts.then_some(Lsn(next.min(durable)));
        let mut local: Vec<u64> = fs
            .list(log_dir)?
            .iter()
            .filter_map(|p| parse_segment(p))
            .collect();
        local.sort_unstable();
        for base in local.into_iter().filter(|&b| b + segment <= durable) {
            if base < next {
                continue;
            }
            if base > next {
                return Err(io_error(inside(log_dir, &segment_name(next)))(
                    IoError::NotFound,
                ));
            }
            self.copy(
                &mut fs,
                &inside(log_dir, &segment_name(base)),
                base,
                published.get(&base),
            )?;
            next = base + segment;
            *checked = next;
            self.status.lock(rt).through = Some(Lsn(next));
        }
        Ok(())
    }

    /// Returns the published segments by base with their hashes, and the epoch file's hash,
    /// removing private leftovers and forcing the directory. Two names of one base fail as damaged.
    fn listed(&self, fs: &mut Fs<R>) -> Io<(BTreeMap<u64, String>, Option<String>)> {
        let mut published = BTreeMap::new();
        let mut epoch = None;
        for path in fs.list(&self.dir)? {
            let name = path.rsplit_once('/').map_or(path.as_str(), |(_, n)| n);
            if name.contains(PARTIAL) {
                fs.remove(&path)?;
            } else if let Some((stem, hash)) = name.split_once('-') {
                if stem == EPOCH {
                    epoch = Some(hash.to_owned());
                } else if let Some(base) = parse_segment(stem)
                    && published.insert(base, hash.to_owned()).is_some()
                {
                    return Err(BackupError::Damaged {
                        part: stem.to_owned(),
                    });
                }
            }
        }
        fs.sync_dir(&self.dir)?;
        Ok((published, epoch))
    }
}

impl<R: Runtime> Archiver<R> {
    /// Publish the epoch file naming the epoch this one branched from and where, unless the
    /// directory holds it. `held` is the hash of the one it holds.
    fn publish_epoch(&self, fs: &mut Fs<R>, held: Option<String>) -> Io<()> {
        let text = &self.epoch;
        if held.as_deref() == Some(&hex(blake3::hash(text).as_bytes())) {
            return Ok(());
        }
        self.put(fs, EPOCH, held.as_ref(), |fs, aside, file| {
            fs.write(aside, file, 0, text)?;
            Ok(blake3::hash(text))
        })
    }

    /// Copy the segment at `path`, starting at `base`, to the destination.
    fn copy(&self, fs: &mut Fs<R>, path: &str, base: u64, held: Option<&String>) -> Io<()> {
        self.put(fs, &segment_name(base), held, |fs, aside, file| {
            read_segment(fs, path, Some((aside, file)))?
                .ok_or_else(|| io_error(path)(IoError::NotFound))
        })
    }

    /// Write `stem` aside with `fill`, which returns the hash of what it wrote, and force it. Then
    /// rename it to `stem-HASH` and force the directory, unless `held` names a published copy: the
    /// same hash leaves it, another fails as damaged.
    fn put(
        &self,
        fs: &mut Fs<R>,
        stem: &str,
        held: Option<&String>,
        fill: impl FnOnce(&mut Fs<R>, &str, FileId) -> Io<blake3::Hash>,
    ) -> Io<()> {
        let (aside, file) = fs.aside_file(&inside(&self.dir, stem))?;
        let hash = fs.closing(&aside, file, |fs| {
            let hash = fill(fs, &aside, file)?;
            fs.sync(&aside, file)?;
            Ok(hex(hash.as_bytes()))
        })?;
        match held {
            None => fs.publish(
                &aside,
                &inside(&self.dir, &format!("{stem}-{hash}")),
                || BackupError::Occupied(stem.to_owned()),
            ),
            Some(held) if *held == hash => fs.remove(&aside),
            Some(held) => {
                fs.remove(&aside)?;
                Err(BackupError::Damaged {
                    part: format!("{stem}-{held}"),
                })
            }
        }
    }
}

/// Read the segment at `path` whole, writing it to `to` when given. Returns its hash, `None` when
/// the log no longer holds it.
fn read_segment<R: Runtime>(
    fs: &mut Fs<R>,
    path: &str,
    to: Option<(&str, FileId)>,
) -> Io<Option<blake3::Hash>> {
    let (source, len) = match fs.open(path, false) {
        Err(BackupError::Io {
            error: IoError::NotFound,
            ..
        }) => return Ok(None),
        opened => opened?,
    };
    fs.closing(path, source, |fs| {
        let mut hash = blake3::Hasher::new();
        let mut at = 0;
        while at < len {
            let n = IO_CHUNK.min(len - at);
            let bytes = fs.read(path, source, at, n as usize)?;
            hash.update(&bytes);
            if let Some((aside, file)) = to {
                fs.write(aside, file, at, &bytes)?;
            }
            at += n;
        }
        Ok(Some(hash.finalize()))
    })
}

/// Recycle `pager`'s log after a pass of `archiver`, unless the database is quarantined whole,
/// and let the log of this open be recycled from then on.
///
/// Preconditions: every reader of the log the open found has run.
pub fn recycle_at_open<R: Runtime>(pager: &Pager<R>, archiver: Option<&Archiver<R>>) {
    if pager.is_quarantined(Quarantine::Database) {
        return;
    }
    if let Some(a) = archiver {
        a.archive();
    }
    pager.recycle_log();
    pager.release_session_log();
}
