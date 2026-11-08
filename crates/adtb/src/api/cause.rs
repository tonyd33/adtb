use std::fmt;

use adtb_lang::norm;
use adtb_lang::permission;
use adtb_lang::span::FileDiag;
use adtb_lang::textual::PrintError;
use adtb_lang::types::Ty;

use crate::catalog::{self, KeptRow};
use crate::exec::{self, Val};
use crate::mvcc::Xid;
use crate::{storage, write};

use super::bundle::BadRow;
use super::error::{Error, OpenError, RejectReason};

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
    Suspended(Xid),
    UnprintableKey(PrintError),
    NoRows,
    NoIdentity(permission::Error),
    NoRequirements(permission::Error),
    AdhocNotNormal(norm::Error),
    CallNotNormal(norm::Error),
    CallNotOneItem,
    RegistrationNotNormal(norm::Error),
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
            Broken::Suspended(on) => {
                write!(
                    f,
                    "a statement suspended on {on:?} outside a resumable call"
                )
            }
            Broken::UnprintableKey(e) => f.write_str(&e.message),
            Broken::NoRows => f.write_str("a command returns its rows"),
            Broken::NoIdentity(e) => write!(f, "a relation of the schema has no identity: {e:?}"),
            Broken::NoRequirements(e) => {
                write!(f, "a checked registered call has no requirements: {e:?}")
            }
            Broken::AdhocNotNormal(e) => write!(f, "checked ad hoc text has no normal form: {e:?}"),
            Broken::CallNotNormal(e) => {
                write!(f, "a checked registered call has no normal form: {e:?}")
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
    Finding(KeptRow),
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
            Corruption::Storage(e) => write!(f, "{e:?}"),
            Corruption::Catalog(e) => f.write_str(&catalog::detail(e)),
            Corruption::Exec(e) => f.write_str(&exec::detail(e)),
            Corruption::Normal(e) => write!(f, "{e:?}"),
            Corruption::Write(e) => f.write_str(&write::detail(e)),
            Corruption::Engine(e) => write!(f, "{e:?}"),
            Corruption::BundleRow(row) => write!(f, "{row}"),
            Corruption::Finding(row) => write!(f, "a finding holds {row:?}"),
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
}

/// Returns the refusal of a file that `cause` makes.
pub(super) fn corrupt(cause: impl Into<Corruption>) -> OpenError {
    OpenError::Corrupt {
        detail: cause.into().to_string(),
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

    /// Returns how many times `text` builds or renames `Internal` or `Corrupt` outside its tests.
    /// A macro that pastes the name together is not seen.
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
            let Some(len) = ["Internal", "Corrupt"]
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
                let arm =
                    next(k + 1).is_some_and(|n| c[n..].starts_with(&['=', '>']) || word(n, "if"));
                found += usize::from(!declared && !arm);
            }
        }
        found
    }

    #[test]
    fn the_search_finds_each_form_of_building_internal_or_corrupt() {
        for bad in [
            "fn f() -> R { R::Internal { what: w } }",
            "fn f() -> R { Internal { what } }",
            "fn f() -> R { R::Internal { what: String::from(x) } }",
            "fn f() -> R { R::Internal { what: String::new() } }",
            "use super::RejectReason::Internal as Bug;",
            "#[cfg(test)]\nmod tests {\n}\nfn f() -> O { O::Corrupt { detail: d } }",
            "fn f() -> O { O::Corrupt {\n    detail: format!(\"{x:?}\"),\n} }",
        ] {
            assert_eq!(1, offences(false, bad), "{bad}");
        }
        for good in [
            "enum R { Internal {\n    what: String,\n}, }",
            "enum O { Corrupt { detail: String }, }",
            "#[cfg(test)]\nmod tests { fn f() -> R { R::Internal { what: w } } }",
            "fn f(r: R) { match r { R::Internal { what } => g(what) } }",
            "fn f() -> &str { \"R::Internal { what }\" } /* Corrupt { detail } */",
        ] {
            assert_eq!(0, offences(true, good), "{good}");
        }
    }

    #[test]
    fn an_error_with_no_value_reads_as_its_value_text() {
        let e = super::Error::rejected(super::RejectReason::Cancelled);
        let text = super::Broken::NoValue(e).to_string();
        assert_eq!("Rejected { reason: Cancelled }", text);
    }

    #[test]
    fn a_catalog_error_holding_a_plan_or_write_cause_reads_as_its_text() {
        use crate::{catalog, plan, write};
        let plan = plan::Error::Internal(plan::Broken::GroupFieldMissing);
        let corruption = super::Corruption::Catalog(catalog::Broken::Plan(plan).into());
        let text = "Internal(\"a broken invariant: a group has a key and rows\")";
        assert_eq!(text, corruption.to_string());
        let admission = catalog::Broken::Admission(write::Broken::ReadOnly);
        let broken = super::Broken::CatalogError(admission.into());
        let text = "Internal(\"a read-only transaction runs no write\")";
        assert_eq!(text, broken.to_string());
    }

    #[test]
    fn only_a_cause_builds_internal_or_corrupt_outside_tests() {
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
        assert_eq!(vec![(src.join("api/cause.rs"), 2)], built);
    }
}
