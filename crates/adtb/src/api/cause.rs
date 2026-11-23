use std::fmt;

use adtb_lang::kernel::Invalid;
use adtb_lang::norm;
use adtb_lang::permission;
use adtb_lang::span::FileDiag;
use adtb_lang::textual::PrintError;
use adtb_lang::types::Ty;

use crate::catalog::{self, KeptField, KeptRow};
use crate::exec::{self, Val};
use crate::runtime::IoError;
use crate::{btree, heap, mvcc, storage, write};

use super::bundle::BadRow;
use super::error::{Error, FatalReason, OpenError, RejectReason};

/// A guarantee of the engine that did not hold, made into the text of `RejectReason::Internal`.
#[derive(Debug)]
pub(super) enum Broken {
    Write(write::Broken),
    Exec(exec::Broken),
    Catalog(catalog::Broken),
    CatalogError(catalog::Error),
    Unchecked {
        built: String,
        messages: Vec<String>,
    },
    UnprintableKey(PrintError),
    NoRows,
    AdhocNoRequirements(permission::Error),
    NoRequirements(permission::Error),
    AdhocNotNormal(norm::Error),
    CallNotNormal(norm::Error),
    /// A registered call's translation to a later version breaks the kernel's invariants.
    TranslationNotNormal(norm::Error),
    CallNotOneItem,
    RegistrationNotNormal(norm::Error),
    /// A term of a checked registered call breaks the kernel's invariants.
    CallIllTyped(Invalid),
    /// The checker refuses a text for a guarantee of its own or of the normaliser that does
    /// not hold.
    Diagnosed(String),
    NotASet(Val),
    UnresolvedGrant {
        privilege: String,
    },
    NoRole {
        id: i64,
        owned: String,
    },
    HostCode(Vec<FileDiag>),
    Lacking(Error),
    Batches(exec::Error),
    NoColumn(Ty),
    /// An error whose value does not read as a value of the prelude sum `Error`.
    NoValue(Error),
}

impl fmt::Display for Broken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Broken::Write(b) => write!(f, "{b}"),
            Broken::Exec(e) => write!(f, "{e}"),
            Broken::Catalog(e) => write!(f, "{e}"),
            Broken::CatalogError(e) => f.write_str(&catalog::detail(e)),
            Broken::Unchecked { built, messages } => {
                write!(f, "{built} does not check: {}", messages.join("; "))
            }
            Broken::UnprintableKey(e) => f.write_str(&e.message),
            Broken::NoRows => f.write_str("a command returns its rows"),
            Broken::AdhocNoRequirements(e) => {
                write!(f, "checked ad hoc text has no requirements: {e:?}")
            }
            Broken::NoRequirements(e) => {
                write!(f, "a checked registered call has no requirements: {e:?}")
            }
            Broken::AdhocNotNormal(e) => write!(f, "checked ad hoc text has no normal form: {e:?}"),
            Broken::CallNotNormal(e) => {
                write!(f, "a checked registered call has no normal form: {e:?}")
            }
            Broken::TranslationNotNormal(e) => {
                write!(
                    f,
                    "a registered call's translation has no normal form: {e:?}"
                )
            }
            Broken::CallNotOneItem => {
                f.write_str("a checked registered call has no normal form: a call is one item")
            }
            Broken::RegistrationNotNormal(e) => {
                write!(
                    f,
                    "a checked registration's program does not normalise: {e:?}"
                )
            }
            Broken::CallIllTyped(e) => write!(
                f,
                "a checked registered call's kernel term {:?} keeps the kernel's invariants: {}",
                e.at(),
                e.reason()
            ),
            Broken::Diagnosed(m) => write!(f, "the checker keeps its own guarantees: {m}"),
            Broken::NotASet(v) => write!(f, "a set-valued item evaluates to a set, not {v:?}"),
            Broken::UnresolvedGrant { privilege } => {
                write!(f, "a resolved `{privilege}` names an object of the schema")
            }
            Broken::NoRole { id, owned } => write!(f, "no role has id {id}, which owns {owned}"),
            Broken::HostCode(ds) => write!(f, "{ds:?}"),
            Broken::Lacking(e) => write!(f, "{e:?}"),
            Broken::Batches(e) => f.write_str(&exec::detail(e)),
            Broken::NoColumn(ty) => write!(f, "a result of type {ty:?} has no column"),
            Broken::NoValue(e) => write!(f, "{}", e.value()),
        }
    }
}

/// A failure of the file or the transactions layer, made into the text of `FatalReason::Storage`.
#[derive(Debug)]
pub(super) enum Failure {
    /// A page that cannot be handed out, its damage mapped to no mark.
    Page(storage::PageError),
    Transactions(mvcc::Error),
    /// The open could not arm the read-in gate.
    Unarmed(storage::Unarmed),
    /// A stored structure a statement reads does not decode.
    Write(write::Rot),
    /// A stored structure or spill file a plan reads does not decode.
    Exec(exec::Rot),
    /// An entry of this index names no version.
    OrphanEntry(u32),
    /// The call's spill file could not be made, written or read.
    Spill(IoError),
    /// A file of a backup, dump or restore, by its path, failed with this error.
    File {
        path: String,
        error: IoError,
    },
    /// The open of a file a backup proves or a restore builds, by its path, failed with this
    /// error.
    Open {
        path: String,
        error: std::io::Error,
    },
    /// The file a restore built, by its path, refused its open.
    Aside {
        path: String,
        refused: OpenError,
    },
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Page(e) => write!(f, "{e}"),
            Failure::Transactions(e) => write!(f, "{e}"),
            Failure::Unarmed(why) => write!(f, "{why}"),
            Failure::Write(r) => write!(f, "{r}"),
            Failure::Exec(r) => write!(f, "{r}"),
            Failure::OrphanEntry(index) => write!(f, "an entry of index {index} names no version"),
            Failure::Spill(e) => write!(f, "the spill file failed: {}", storage::io_text(*e)),
            Failure::File { path, error } => write!(f, "`{path}`: {}", storage::io_text(*error)),
            Failure::Open { path, error } => write!(f, "`{path}`: {error}"),
            Failure::Aside { path, refused } => write!(f, "`{path}`: {refused}"),
        }
    }
}

impl Error {
    /// Returns the failure of the file or the transactions layer that `cause` states.
    pub(super) fn storage(cause: Failure) -> Error {
        Error::Fatal {
            reason: FatalReason::Storage {
                detail: cause.to_string(),
            },
        }
    }
}

/// What makes a file one the engine refuses to open, made into the text of `OpenError::Corrupt`.
#[derive(Debug)]
pub(super) enum Corruption {
    Storage(storage::OpenError),
    Catalog(catalog::Error),
    Exec(exec::Error),
    Normal(norm::Error),
    Write(write::Error),
    Engine(Error),
    BundleRow(BadRow),
    Finding(KeptRow, catalog::Rot),
    NoSchemaOwner,
}

impl fmt::Display for Corruption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Corruption::Storage(storage::OpenError::Exists) => {
                f.write_str("neither copy of the file header verifies")
            }
            Corruption::Storage(storage::OpenError::LogPresent) => {
                f.write_str("the log directory holds a log that no file here wrote")
            }
            Corruption::Storage(storage::OpenError::Reserved) => {
                f.write_str("the reserved directory of a create holds what no create left")
            }
            Corruption::Storage(storage::OpenError::OtherLayout(layout)) => write!(
                f,
                "the file header has layout {layout}, not one this engine reads"
            ),
            Corruption::Storage(e) => write!(f, "{e}"),
            Corruption::Catalog(e) => f.write_str(&catalog::detail(e)),
            Corruption::Exec(e) => f.write_str(&exec::detail(e)),
            Corruption::Normal(e) => write!(f, "{e}"),
            Corruption::Write(e) => f.write_str(&write::detail(e)),
            Corruption::Engine(e) => write!(f, "{e}"),
            Corruption::BundleRow(row) => write!(f, "{row}"),
            Corruption::Finding(row, rot) => {
                write!(f, "a kept row of group `{}`", row.group)?;
                match row.ctor {
                    Some(id) => write!(f, " and constructor {id} holds (")?,
                    None => f.write_str(" and no constructor holds (")?,
                }
                for (at, field) in row.fields.iter().enumerate() {
                    let sep = if at == 0 { "" } else { ", " };
                    match field {
                        KeptField::Int(n) => write!(f, "{sep}{n}")?,
                        KeptField::Instant(t) => write!(f, "{sep}instant {t}")?,
                        KeptField::Text(t) => write!(f, "{sep}`{t}`")?,
                    }
                }
                write!(f, "): {rot}")
            }
            Corruption::NoSchemaOwner => {
                f.write_str("a nonempty roles relation has no schema owner")
            }
        }
    }
}

macro_rules! corruption_of {
    ($($variant:ident($t:ty)),*) => {$(
        impl From<$t> for Corruption {
            fn from(e: $t) -> Corruption {
                Corruption::$variant(e)
            }
        }
    )*};
}

corruption_of!(
    Storage(storage::OpenError),
    Catalog(catalog::Error),
    Exec(exec::Error),
    Normal(norm::Error),
    Write(write::Error),
    Engine(Error),
    BundleRow(BadRow)
);

/// Returns the refusal of `Internal` that `cause` makes.
pub(super) fn internal(cause: Broken) -> RejectReason {
    RejectReason::Internal {
        what: cause.to_string(),
    }
}

impl Error {
    pub(super) fn internal(cause: Broken) -> Error {
        Error::rejected(internal(cause))
    }

    /// Returns the refusal of a text that expands past this build's limits.
    pub(super) fn too_large() -> Error {
        Error::invalid(norm::Cause::TooLarge.to_string())
    }

    /// Returns what a body that has no normal form for `e` answers: the refusal past this build's
    /// limits, or the internal error `broken` makes of `e`.
    pub(super) fn not_normal(e: norm::Error, broken: impl FnOnce(norm::Error) -> Broken) -> Error {
        match e.cause {
            norm::Cause::TooLarge => Error::too_large(),
            norm::Cause::Stuck(_) | norm::Cause::Invalid(..) => Error::internal(broken(e)),
        }
    }

    /// Returns what a body whose requirements fail with `e` answers, as [`Error::not_normal`].
    pub(super) fn unanalysed(
        e: permission::Error,
        broken: impl FnOnce(permission::Error) -> Broken,
    ) -> Error {
        if past_limits(&e) {
            Error::too_large()
        } else {
            Error::internal(broken(e))
        }
    }
}

/// Returns whether the permission analysis fails with `e` because its text expands past this
/// build's limits.
pub(super) fn past_limits(e: &permission::Error) -> bool {
    matches!(
        e,
        permission::Error::TooLarge
            | permission::Error::Normalization(norm::Error {
                cause: norm::Cause::TooLarge,
                ..
            })
    )
}

/// Returns the failure of an open given migrations while it could not arm the read-in gate.
pub(super) fn unarmed(why: storage::Unarmed) -> OpenError {
    OpenError::Io(std::io::Error::other(Io(Failure::Unarmed(why))))
}

/// Returns the refusal of a file that `cause` makes: [`OpenError::PastLimits`] for a stored body
/// past this build's limits, and [`OpenError::Io`] for a page the device refused to read and for
/// a spill file that failed.
pub(super) fn corrupt(cause: impl Into<Corruption>) -> OpenError {
    match cause.into() {
        Corruption::Catalog(catalog::Error::PastLimits(object)) => OpenError::PastLimits { object },
        Corruption::Catalog(catalog::Error::FormatTooOld(format)) => {
            OpenError::FormatTooOld { format }
        }
        Corruption::Catalog(catalog::Error::Spill(io)) => {
            OpenError::Io(std::io::Error::other(Io(Failure::Spill(io))))
        }
        cause => match cause.page() {
            Some(e) if e.damage == storage::Damage::Unreadable => {
                OpenError::Io(std::io::Error::other(Io(Failure::Page(e))))
            }
            _ => OpenError::Corrupt {
                detail: cause.to_string(),
            },
        },
    }
}

/// A storage failure as the I/O failure of an open.
#[derive(Debug)]
struct Io(Failure);

impl fmt::Display for Io {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Io {}

impl Corruption {
    /// Returns the page whose refusal this is, `None` when it is no page's.
    fn page(&self) -> Option<storage::PageError> {
        match self {
            Corruption::Storage(
                storage::OpenError::Page(e) | storage::OpenError::Unformatted(e),
            ) => Some(*e),
            Corruption::Catalog(e) => catalog_page(e),
            Corruption::Exec(e) | Corruption::Write(write::Error::Exec(e)) => exec_page(e),
            Corruption::Write(write::Error::Transactions(e)) => transactions_page(e),
            Corruption::Storage(_)
            | Corruption::Normal(_)
            | Corruption::Write(_)
            | Corruption::Engine(_)
            | Corruption::BundleRow(_)
            | Corruption::Finding(..)
            | Corruption::NoSchemaOwner => None,
        }
    }
}

fn catalog_page(e: &catalog::Error) -> Option<storage::PageError> {
    match e {
        catalog::Error::Page(e)
        | catalog::Error::Heap(heap::Error::Page(e))
        | catalog::Error::Index(btree::Error::Page(e)) => Some(*e),
        catalog::Error::Txn(e) => transactions_page(e),
        _ => None,
    }
}

fn exec_page(e: &exec::Error) -> Option<storage::PageError> {
    match e {
        exec::Error::Page(e) => Some(*e),
        _ => None,
    }
}

fn transactions_page(e: &mvcc::Error) -> Option<storage::PageError> {
    match e {
        mvcc::Error::Corrupt(e) | mvcc::Error::History(btree::Error::Page(e)) => Some(*e),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy();
            if path.is_dir() {
                if name != "tests" {
                    sources(&path, out);
                }
            } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
                out.push(path);
            }
        }
    }

    /// Returns `text` with comments and literals blanked and each `#[cfg(test)]` item or field
    /// removed.
    fn code(text: &str) -> Vec<char> {
        let mut c: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < c.len() {
            let raw = c[i] == 'r'
                && (i == 0 || !ident(c[i - 1]))
                && c[i + 1..]
                    .iter()
                    .position(|&x| x != '#')
                    .map(|n| c[i + 1 + n])
                    == Some('"');
            let end = if c[i..].starts_with(&['/', '/']) {
                c[i..]
                    .iter()
                    .position(|&x| x == '\n')
                    .map_or(c.len(), |n| i + n)
            } else if c[i..].starts_with(&['/', '*']) {
                (i + 2..c.len())
                    .find(|&j| c[j..].starts_with(&['*', '/']))
                    .unwrap()
                    + 2
            } else if raw {
                let hashes = c[i + 1..].iter().take_while(|&&x| x == '#').count();
                let close: Vec<char> = std::iter::once('"').chain(vec!['#'; hashes]).collect();
                (i + hashes + 2..c.len())
                    .find(|&j| c[j..].starts_with(&close))
                    .unwrap()
                    + hashes
                    + 1
            } else if c[i] == '"' {
                let mut j = i + 1;
                while c[j] != '"' {
                    j += if c[j] == '\\' { 2 } else { 1 };
                }
                j + 1
            } else if c[i] == '\'' && c.get(i + 1) == Some(&'\\') {
                (i + 2..c.len()).find(|&j| c[j] == '\'').unwrap() + 1
            } else if c[i] == '\'' && c.get(i + 2) == Some(&'\'') {
                i + 3
            } else {
                i + 1
            };
            if end > i + 1 {
                c[i..end].fill(' ');
            }
            i = end;
        }
        let gate: Vec<char> = "#[cfg(test)]".chars().collect();
        while let Some(at) = (0..c.len()).find(|&j| c[j..].starts_with(&gate)) {
            let (mut depth, mut j) = (0, at + gate.len());
            loop {
                match c[j] {
                    ';' | ',' if depth == 0 => break,
                    '}' if depth == 0 => {
                        j -= 1;
                        break;
                    }
                    '}' if depth == 1 => break,
                    '{' | '(' | '[' => depth += 1,
                    '}' | ')' | ']' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            c[at..=j].fill(' ');
        }
        c
    }

    fn ident(x: char) -> bool {
        x.is_alphanumeric() || x == '_'
    }

    /// Returns how many times `text` builds or renames `Internal`, `Corrupt` or `Storage` outside
    /// its tests. A pattern whose fields end in `..` builds nothing. A macro that pastes the name
    /// together is not seen.
    fn offences(declares: bool, text: &str) -> usize {
        let c = code(text);
        let word = |at: usize, w: &str| {
            let w: Vec<char> = w.chars().collect();
            c[at..].starts_with(&w)
                && (at == 0 || !ident(c[at - 1]))
                && c.get(at + w.len()).is_none_or(|&x| !ident(x))
        };
        let next = |from: usize| (from..c.len()).find(|&j| !c[j].is_whitespace());
        let mut found = 0;
        for at in 0..c.len() {
            let Some(len) = ["Internal", "Corrupt", "Storage"]
                .iter()
                .find(|w| word(at, w))
                .map(|w| w.len())
            else {
                continue;
            };
            let Some(j) = next(at + len) else { continue };
            if word(j, "as") {
                found += 1;
            } else if c[j] == '{' {
                let mut k = j;
                let mut depth = 0;
                while depth != 1 || c[k] != '}' {
                    depth += i32::from(c[k] == '{') - i32::from(c[k] == '}');
                    k += 1;
                }
                let fields: String = c[j + 1..k].iter().filter(|x| !x.is_whitespace()).collect();
                let declared = declares && (fields == "what:String," || fields == "detail:String");
                let arm = fields.ends_with("..")
                    || next(k + 1)
                        .is_some_and(|n| c[n..].starts_with(&['=', '>']) || word(n, "if"));
                found += usize::from(!declared && !arm);
            }
        }
        found
    }

    #[test]
    fn the_search_finds_each_form_of_building_internal_corrupt_or_storage() {
        for bad in [
            "fn f() -> R { R::Internal { what: w } }",
            "fn f() -> R { Internal { what } }",
            "fn f() -> R { R::Internal { what: String::from(x) } }",
            "fn f() -> R { R::Internal { what: String::new() } }",
            "use super::RejectReason::Internal as Bug;",
            "#[cfg(test)]\nmod tests {\n}\nfn f() -> O { O::Corrupt { detail: d } }",
            "fn f() -> O { O::Corrupt {\n    detail: format!(\"{x:?}\"),\n} }",
            "fn f() -> F { F::Storage { detail: d.to_string() } }",
        ] {
            assert_eq!(1, offences(false, bad), "{bad}");
        }
        for good in [
            "enum R { Internal {\n    what: String,\n}, }",
            "enum O { Corrupt { detail: String }, }",
            "#[cfg(test)]\nmod tests { fn f() -> R { R::Internal { what: w } } }",
            "fn f(r: R) { match r { R::Internal { what } => g(what) } }",
            "fn f(e: E) -> bool { matches!(e, E::Fatal { reason: F::Storage { .. } }) }",
            "enum F { Storage { detail: String }, }",
            "fn f() -> &str { \"R::Internal { what }\" } /* Corrupt { detail } */",
        ] {
            assert_eq!(0, offences(true, good), "{good}");
        }
    }

    #[test]
    fn only_a_cause_builds_internal_corrupt_or_storage_outside_tests() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        sources(&src, &mut files);
        let built: Vec<(PathBuf, usize)> = files
            .into_iter()
            .map(|path| {
                let text = std::fs::read_to_string(&path).unwrap();
                let n = offences(path.ends_with("api/error.rs"), &text);
                (path, n)
            })
            .filter(|(_, n)| *n > 0)
            .collect();
        assert_eq!(vec![(src.join("api/cause.rs"), 3)], built);
    }
}
