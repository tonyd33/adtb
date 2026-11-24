use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Mutex;

use super::*;

/// The most schemas one memo holds.
pub(super) const BOUND: usize = 32;

/// The inputs of a build of declarations before their ids are relabelled: the declarations,
/// declaration `i` file `i`, every user module, and the relation variables a later operator of
/// the migration being applied adds.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct BuildInputs {
    decls: Vec<SourceDecl>,
    modules: BTreeSet<String>,
    ahead: Vec<schema::Ahead>,
}

/// A build of [`BuildInputs`]: the schema, its ids those of declaration order, and the files it
/// was built from, file `i` holding declaration `i`, then a file of each module that holds
/// nothing.
#[derive(Clone)]
pub(super) struct Prebuilt {
    schema: Arc<Schema>,
    files: Arc<[(FileId, ast::SchemaFile)]>,
}

impl Prebuilt {
    pub(super) fn schema(&self) -> &Arc<Schema> {
        &self.schema
    }

    pub(super) fn files(&self) -> &Arc<[(FileId, ast::SchemaFile)]> {
        &self.files
    }
}

/// Whose memo a build is looked up in.
#[derive(Clone, Copy, Debug)]
pub(super) enum Owner {
    Source,
    Model,
}

impl BuildInputs {
    pub(super) fn new(
        decls: Vec<SourceDecl>,
        modules: BTreeSet<String>,
        ahead: Vec<schema::Ahead>,
    ) -> BuildInputs {
        BuildInputs {
            decls,
            modules,
            ahead,
        }
    }

    /// Returns the text of each declaration's file, module header included.
    pub(super) fn texts(&self) -> Vec<String> {
        self.decls
            .iter()
            .map(|d| format!("{}{}", header(&d.module), d.text))
            .collect()
    }

    /// Returns the files of the declarations and of each module that holds nothing, parsed
    /// through `parsed`.
    pub(super) fn files(
        &self,
        parsed: &ParsedMemo,
    ) -> Result<Vec<(FileId, ast::SchemaFile)>, Vec<FileDiag>> {
        let mut files = parsed.files(&self.decls)?;
        for m in &self.modules {
            if !files.iter().any(|(_, f)| f.module.text == *m) {
                let empty = parse_schema(&header(m), Options::default())
                    .ok()
                    .expect("a module header parses");
                files.push((FileId::EMPTY_MODULE, empty));
            }
        }
        Ok(files)
    }

    /// Build the declarations, their ids those of declaration order, parsing them through
    /// `parsed`.
    pub(super) fn build(&self, parsed: &ParsedMemo) -> Result<Prebuilt, Vec<FileDiag>> {
        let files = self.files(parsed)?;
        let schema = schema::build_ahead(&files, &self.ahead)?;
        Ok(Prebuilt {
            schema: Arc::new(schema),
            files: files.into(),
        })
    }
}

struct Built {
    inputs: BuildInputs,
    built: Prebuilt,
}

/// A bounded memo of builds by their inputs, most recently used first.
///
/// A source and its clones share one. So do a model and its clones from the start of one
/// migration run, which takes a new memo, to the start of the next: the run's operators, the
/// models after each, its probes and reverts, and the sources replayed from them.
#[derive(Clone, Default)]
pub(super) struct BuiltMemo(Arc<Mutex<VecDeque<Built>>>);

impl fmt::Debug for BuiltMemo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BuiltMemo")
    }
}

impl BuiltMemo {
    fn entries(&self) -> std::sync::MutexGuard<'_, VecDeque<Built>> {
        self.0
            .lock()
            .expect("no panic while the memo's lock is held: it only compares and moves entries")
    }

    pub(super) fn get(&self, inputs: &BuildInputs) -> Option<Prebuilt> {
        let mut entries = self.entries();
        let at = entries.iter().position(|b| b.inputs == *inputs)?;
        let hit = entries.remove(at)?;
        let built = hit.built.clone();
        entries.push_front(hit);
        Some(built)
    }

    pub(super) fn put(&self, inputs: BuildInputs, built: Prebuilt) {
        let mut entries = self.entries();
        entries.push_front(Built { inputs, built });
        entries.truncate(BOUND);
    }

    /// Returns the build of `inputs`, from the memo of `owner` or built through `parsed` and
    /// kept.
    #[cfg_attr(
        not(any(test, feature = "testing")),
        expect(
            unused_variables,
            reason = "only the samplers of a test build read `owner`; a shipped reader would use it"
        )
    )]
    pub(super) fn built(
        &self,
        inputs: BuildInputs,
        parsed: &ParsedMemo,
        owner: Owner,
    ) -> Result<Prebuilt, Vec<FileDiag>> {
        if let Some(hit) = self.get(&inputs) {
            #[cfg(any(test, feature = "testing"))]
            {
                let sampler = match owner {
                    Owner::Source => &crate::sampled::BUILDS,
                    Owner::Model => &crate::sampled::MODEL_BUILDS,
                };
                if sampler.due() {
                    same_as_uncached(&inputs, &hit);
                }
            }
            return Ok(hit);
        }
        let built = inputs.build(parsed)?;
        self.put(inputs, built.clone());
        Ok(built)
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries().len()
    }
}

/// Panic unless `hit` is what a build of `inputs` gives with no memo.
#[cfg(any(test, feature = "testing"))]
fn same_as_uncached(inputs: &BuildInputs, hit: &Prebuilt) {
    let uncached = inputs
        .build(&ParsedMemo::default())
        .expect("a build the memo holds builds");
    assert_eq!(
        (format!("{:?}", uncached.schema), &uncached.files[..]),
        (format!("{:?}", hit.schema), &hit.files[..]),
        "a memoised build equals an uncached one of its inputs {:?}",
        inputs.decls
    );
}

/// A memo of the module files a source and its clones parsed, by their text, emptied when it is
/// full.
#[derive(Clone, Default)]
pub(super) struct ParsedMemo(Arc<Mutex<HashMap<String, ast::SchemaFile>>>);

/// The most files one parse memo holds.
const PARSED_BOUND: usize = 256;

impl fmt::Debug for ParsedMemo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ParsedMemo")
    }
}

impl ParsedMemo {
    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, ast::SchemaFile>> {
        self.0
            .lock()
            .expect("no panic while the memo's lock is held: it only looks up and moves entries")
    }

    /// Returns the parse of module file `text`, as `parse_schema` with elisions gives it.
    pub(super) fn parsed(&self, text: &str) -> Result<ast::SchemaFile, Vec<Diag>> {
        if let Some(f) = self.entries().get(text) {
            return Ok(f.clone());
        }
        let f = parse_schema(text, Options { elisions: true }).ok()?;
        let mut entries = self.entries();
        if entries.len() >= PARSED_BOUND {
            entries.clear();
        }
        entries.insert(text.to_owned(), f.clone());
        Ok(f)
    }

    /// Returns the file of each declaration, file `i` holding declaration `i`, as
    /// [`ParsedMemo::parsed`] gives it.
    pub(super) fn files(
        &self,
        decls: &[SourceDecl],
    ) -> Result<Vec<(FileId, ast::SchemaFile)>, Vec<FileDiag>> {
        let mut out = Vec::new();
        let mut errs = Vec::new();
        for (i, d) in decls.iter().enumerate() {
            match self.parsed(&format!("{}{}", header(&d.module), d.text)) {
                Ok(f) => out.push((FileId(i as u32), f)),
                Err(ds) => errs.extend(ds.into_iter().map(|diag| FileDiag {
                    file: FileId(i as u32),
                    diag,
                })),
            }
        }
        if errs.is_empty() { Ok(out) } else { Err(errs) }
    }
}
