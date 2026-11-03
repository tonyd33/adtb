//! Building a database in a reserved directory beside its own names, and publishing it.

use crate::runtime::{AlignedBuf, Done, IoError, Op, Runtime};

use super::Lsn;
use super::block::{BlockState, read_block};
use super::crc::crc32c;
use super::driver::Driver;
use super::log::parse_segment;
use super::pager::{OpenError, Options, Pager, check_options, parent};

const MAGIC: &[u8; 8] = b"adtbmake";
const MARKER: usize = MAGIC.len() + 16 + 4;

/// Lock the reserved file at a path before create removes or renames it.
pub(crate) type Hold<'a> = &'a mut dyn FnMut(&str) -> Result<(), OpenError>;

/// Returns the reserved directory a create of the file `name` builds in.
pub fn creating(name: &str) -> String {
    format!("{name}.creating")
}

/// Returns the path of the marker naming the create that owns [`creating`].
pub fn marker(name: &str) -> String {
    format!("{}/id", creating(name))
}

/// Returns the path a create whose id is `id` builds the file `name` at.
pub fn aside(name: &str, id: &[u8; 16]) -> String {
    let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}/{hex}", creating(name))
}

fn log_of(path: &str) -> String {
    format!("{path}-log")
}

/// Returns the bytes of a marker naming `id`.
pub fn marker_bytes(id: &[u8; 16]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(id);
    out.extend_from_slice(&crc32c(&out).to_le_bytes());
    out
}

/// What a marker holds.
enum Marked {
    /// Nothing written: absent, empty, or one block of zeros.
    Unwritten,
    Whole([u8; 16]),
    Torn,
}

/// Returns what a marker of `len` bytes holds, given its first block `bytes`.
fn marked(len: u64, bytes: &[u8]) -> Marked {
    let (head, sum) = bytes[..MARKER].split_at(MARKER - 4);
    if len == 0 || (len == bytes.len() as u64 && bytes.iter().all(|&b| b == 0)) {
        Marked::Unwritten
    } else if head.starts_with(MAGIC) && crc32c(head).to_le_bytes() == sum {
        Marked::Whole(head[MAGIC.len()..].try_into().expect("an id is 16 bytes"))
    } else {
        Marked::Torn
    }
}

/// What [`create`] made.
pub(crate) enum Created<R: Runtime> {
    /// An earlier create of the name, finished and opened.
    Finished(Pager<R>),
    /// A new database in the reserved directory, and what publishes it.
    Built(Pager<R>, Unpublished),
}

/// A database built in the reserved directory and not yet at its own names.
#[derive(Debug)]
pub(crate) struct Unpublished {
    name: String,
    id: [u8; 16],
    /// The file at `name` is empty and the published one replaces it.
    replace: bool,
}

/// Settle what an earlier create of `options.name` left, then build a new database in the
/// reserved directory, or finish the earlier create when only its file's rename is left.
///
/// Preconditions:
/// - No other create of `options.name` runs.
/// - `options` keeps every limit of the file format.
///
/// Postconditions:
/// - Nothing outside the reserved directory is removed, and in it only what a marker whole and
///   held names: the marker's own file and log directory, then the marker.
/// - A reserved directory holding anything else answers `Reserved`, with nothing removed.
pub(crate) fn create<R: Runtime>(
    rt: R,
    options: &Options,
    hold: Hold<'_>,
) -> Result<Created<R>, OpenError> {
    check_options(options);
    let mut drv = Driver::new(rt.clone());
    let name = &options.name;
    let replace = match drv.open(name, false) {
        Ok((file, len)) => {
            drv.close(file);
            if len > 0 {
                return Err(OpenError::Exists);
            }
            true
        }
        Err(IoError::NotFound) => false,
        Err(e) => return Err(OpenError::Io(e)),
    };
    if let Some(earlier) = settle(&mut drv, options, hold)? {
        let published = Unpublished {
            name: name.clone(),
            id: earlier,
            replace,
        };
        published.publish_file(&mut drv)?;
        return Pager::open(rt, options).map(|(pager, _)| Created::Finished(pager));
    }
    let mut id = [0; 16];
    rt.fill(&mut id);
    let new = Unpublished {
        name: name.clone(),
        id,
        replace,
    };
    new.build(&mut drv, options)?;
    let aside = Options {
        name: aside(name, &id),
        ..options.clone()
    };
    let (data, _) = drv.create_new(&aside.name).map_err(OpenError::Io)?;
    drv.sync_dir(&creating(name));
    match Pager::format_new(rt.clone(), &aside, data, id) {
        Ok(pager) => Ok(Created::Built(pager, new)),
        Err(e) => {
            new.discard(rt, hold)?;
            Err(e)
        }
    }
}

impl Unpublished {
    /// Write and force a marker naming this create's id in the reserved directory, which holds
    /// nothing else.
    fn build<R: Runtime>(&self, drv: &mut Driver<R>, options: &Options) -> Result<(), OpenError> {
        let reserved = creating(&self.name);
        drv.sync_dir(parent(&reserved));
        let (file, _) = drv.open(&marker(&self.name), true).map_err(OpenError::Io)?;
        let mut block = vec![0; options.log_block as usize];
        block[..MARKER].copy_from_slice(&marker_bytes(&self.id));
        drv.write_all(vec![(file, 0, AlignedBuf::from_slice(&block))]);
        drv.sync_all(&[file]);
        drv.close(file);
        drv.sync_dir(&reserved);
        Ok(())
    }

    fn aside(&self) -> String {
        aside(&self.name, &self.id)
    }

    /// Force `pager`'s log, rename the log directory and then the file, the commit point, out of
    /// the reserved directory to their own names, and remove the marker and the directory.
    /// Point the pager at the log's own name.
    ///
    /// Postconditions:
    /// - On failure the file is not at its own name, and the log directory is there only when
    ///   moving it back failed; `LogPresent` when another took the log directory's name.
    /// - Nothing after the file's rename answers an error.
    pub fn publish<R: Runtime>(&self, pager: &Pager<R>, hold: Hold<'_>) -> Result<(), OpenError> {
        pager.flush_log();
        let aside = self.aside();
        let log = log_of(&self.name);
        hold(&aside)?;
        let published = pager.io(|drv| {
            rename(drv, &log_of(&aside), &log, false).map_err(|e| match e {
                IoError::Exists => OpenError::LogPresent,
                e => OpenError::Io(e),
            })?;
            self.force(drv);
            let filed = self.publish_file(drv);
            if filed.is_err() && rename(drv, &log, &log_of(&aside), false).is_ok() {
                self.force(drv);
            }
            filed
        });
        if published.is_ok() {
            pager.segments.lock(&pager.rt).dir = log;
        }
        published
    }

    /// Rename the file out of the reserved directory to its own name, force both directories,
    /// and remove the marker and the reserved directory.
    fn publish_file<R: Runtime>(&self, drv: &mut Driver<R>) -> Result<(), OpenError> {
        rename(drv, &self.aside(), &self.name, self.replace).map_err(OpenError::Io)?;
        let reserved = creating(&self.name);
        self.force(drv);
        if remove(drv, &marker(&self.name)).is_ok() && remove_dir(drv, &reserved).is_ok() {
            drv.sync_dir(parent(&reserved));
        }
        Ok(())
    }

    fn force<R: Runtime>(&self, drv: &mut Driver<R>) {
        let reserved = creating(&self.name);
        drv.sync_dir(&reserved);
        drv.sync_dir(parent(&reserved));
    }

    /// Remove what this create built, its pager closed, unless its log directory is at its own
    /// name already, which a later create rolls forward.
    pub fn discard<R: Runtime>(&self, rt: R, hold: Hold<'_>) -> Result<(), OpenError> {
        let mut drv = Driver::new(rt);
        let entries = listed(&mut drv, &creating(&self.name))?.unwrap_or_default();
        if !entries.contains(&log_of(&self.aside())) {
            return Ok(());
        }
        hold(&self.aside())?;
        remove_built(&mut drv, &self.name, &self.id, &entries)?;
        remove_dir(&mut drv, &creating(&self.name))?;
        drv.sync_dir(parent(&self.name));
        Ok(())
    }
}

/// Settle the reserved directory an earlier create of `options.name` left, making it when
/// absent. Returns the id of an earlier create whose log directory is published, which only its
/// file's rename finishes; else leaves the directory empty.
fn settle<R: Runtime>(
    drv: &mut Driver<R>,
    options: &Options,
    hold: Hold<'_>,
) -> Result<Option<[u8; 16]>, OpenError> {
    let (name, reserved) = (&options.name, creating(&options.name));
    match drv
        .attempt(Op::CreateDir {
            dir: reserved.clone(),
        })
        .result
    {
        Ok(Done::Created) => return Ok(None),
        Ok(_) => {}
        Err(IoError::Exists) => return Err(OpenError::Reserved),
        Err(e) => return Err(OpenError::Io(e)),
    }
    let mark = marker(name);
    let marked = read_marker(drv, options)?;
    let entries = listed(drv, &reserved)?.unwrap_or_default();
    let id = match marked {
        Marked::Unwritten if entries.iter().all(|e| *e == mark) => return Ok(None),
        Marked::Whole(id) => id,
        Marked::Unwritten | Marked::Torn => return Err(OpenError::Reserved),
    };
    let file = aside(name, &id);
    hold(&file)?;
    let entries = listed(drv, &reserved)?.unwrap_or_default();
    let has_log = entries.contains(&log_of(&file));
    if entries.contains(&file) && !has_log && carries(drv, options, &id)? {
        return Ok(Some(id));
    }
    remove_built(drv, name, &id, &entries)?;
    Ok(None)
}

fn read_marker<R: Runtime>(drv: &mut Driver<R>, options: &Options) -> Result<Marked, OpenError> {
    match drv.open(&marker(&options.name), false) {
        Ok((file, len)) => {
            let read = drv.read(file, 0, options.log_block as usize);
            drv.close(file);
            Ok(marked(len, &read.map_err(OpenError::Io)?))
        }
        Err(IoError::NotFound) => Ok(Marked::Unwritten),
        Err(e) => Err(OpenError::Io(e)),
    }
}

/// Answer `LogPresent` when the file at `options.name` is empty or absent and a log directory
/// is at its own name whose first block carries no id a whole marker names.
pub(crate) fn refuse_foreign_log<R: Runtime>(rt: R, options: &Options) -> Result<(), OpenError> {
    let mut drv = Driver::new(rt);
    match listed(&mut drv, &options.log_dir()) {
        Ok(None) => return Ok(()),
        Ok(Some(_)) | Err(OpenError::Io(IoError::NotADirectory)) => {}
        Err(e) => return Err(e),
    }
    match drv.open(&options.name, false) {
        Ok((file, len)) => {
            drv.close(file);
            if len > 0 {
                return Ok(());
            }
        }
        Err(IoError::NotFound) => {}
        Err(e) => return Err(OpenError::Io(e)),
    }
    let owned = match read_marker(&mut drv, options)? {
        Marked::Whole(id) => carries(&mut drv, options, &id)?,
        Marked::Unwritten | Marked::Torn => false,
    };
    if owned {
        Ok(())
    } else {
        Err(OpenError::LogPresent)
    }
}

/// Remove the file, the log directory and then the marker of the create of `name` whose id is
/// `id`, given the reserved directory's `entries`.
///
/// Postconditions: unless every entry is one of those and the log directory holds only
/// segments, answers `Reserved` with nothing removed.
fn remove_built<R: Runtime>(
    drv: &mut Driver<R>,
    name: &str,
    id: &[u8; 16],
    entries: &[String],
) -> Result<(), OpenError> {
    let (mark, file) = (marker(name), aside(name, id));
    let log = log_of(&file);
    if entries
        .iter()
        .any(|e| *e != mark && *e != file && *e != log)
    {
        return Err(OpenError::Reserved);
    }
    let segments = listed(drv, &log)?.unwrap_or_default();
    if segments.iter().any(|s| parse_segment(s).is_none()) {
        return Err(OpenError::Reserved);
    }
    for segment in &segments {
        remove(drv, segment)?;
    }
    if entries.contains(&log) {
        remove_dir(drv, &log)?;
    }
    if entries.contains(&file) {
        remove(drv, &file)?;
    }
    drv.sync_dir(&creating(name));
    if entries.contains(&mark) {
        remove(drv, &mark)?;
    }
    Ok(())
}

/// Returns whether the log directory at `options.name`'s own name begins with a block of the
/// create whose id is `id`.
fn carries<R: Runtime>(
    drv: &mut Driver<R>,
    options: &Options,
    id: &[u8; 16],
) -> Result<bool, OpenError> {
    let first = format!("{}/{:016x}", options.log_dir(), 0);
    let file = match drv.open(&first, false) {
        Ok((file, _)) => file,
        Err(IoError::NotFound) => return Ok(false),
        Err(e) => return Err(OpenError::Io(e)),
    };
    let block = drv.read(file, 0, options.log_block as usize);
    drv.close(file);
    let block = block.map_err(OpenError::Io)?;
    Ok(matches!(read_block(Lsn(0), &block), BlockState::Valid(h, _) if h.file_id == *id))
}

fn rename<R: Runtime>(
    drv: &mut Driver<R>,
    from: &str,
    to: &str,
    replace: bool,
) -> Result<(), IoError> {
    let (from, to) = (from.to_owned(), to.to_owned());
    run(drv, Op::Rename { from, to, replace })
}

fn run<R: Runtime>(drv: &mut Driver<R>, op: Op) -> Result<(), IoError> {
    drv.attempt(op).result.map(drop)
}

fn remove<R: Runtime>(drv: &mut Driver<R>, path: &str) -> Result<(), OpenError> {
    let path = path.to_owned();
    run(drv, Op::Remove { path }).map_err(OpenError::Io)
}

fn remove_dir<R: Runtime>(drv: &mut Driver<R>, path: &str) -> Result<(), OpenError> {
    let path = path.to_owned();
    run(drv, Op::RemoveDir { path }).map_err(OpenError::Io)
}

/// Returns the entries of `dir`, `None` when it is absent.
fn listed<R: Runtime>(drv: &mut Driver<R>, dir: &str) -> Result<Option<Vec<String>>, OpenError> {
    let dir = dir.to_owned();
    match drv.attempt(Op::List { dir }).result {
        Ok(Done::Listed(paths)) => Ok(Some(paths)),
        Err(IoError::NotFound) => Ok(None),
        Err(e) => Err(OpenError::Io(e)),
        Ok(other) => unreachable!("list completed as {other:?}"),
    }
}
