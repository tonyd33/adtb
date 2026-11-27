use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::step::{self, say};

mod agents;
#[cfg(test)]
mod tests;

pub const STEP: &str = "repo";

/// The files git tracks under one checkout, each path relative to its root.
pub struct Tree {
    root: PathBuf,
    files: BTreeSet<PathBuf>,
    dirs: BTreeSet<PathBuf>,
}

impl Tree {
    /// Returns the files `git ls-files` lists in the checkout at `root`.
    pub fn tracked(root: &Path) -> Result<Self, String> {
        let listed = step::output(
            STEP,
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["ls-files", "-z"]),
        )?;
        let files: BTreeSet<PathBuf> = listed
            .split(|&byte| byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(OsStr::from_bytes(path)))
            .collect();
        let dirs = files
            .iter()
            .flat_map(|file| file.ancestors().skip(1))
            .map(Path::to_path_buf)
            .collect();
        Ok(Self {
            root: root.to_path_buf(),
            files,
            dirs,
        })
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.contains(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.contains(path)
    }

    fn holds(&self, path: &Path) -> bool {
        self.is_file(path) || self.is_dir(path)
    }

    /// Returns each tracked file and directory strictly under the directory `dir`.
    fn under(&self, dir: &Path) -> Vec<&Path> {
        self.dirs
            .iter()
            .chain(&self.files)
            .filter(|path| path.as_path() != dir && path.starts_with(dir))
            .map(PathBuf::as_path)
            .collect()
    }

    fn bytes(&self, path: &Path) -> Result<Vec<u8>, String> {
        if !self.is_file(path) {
            return Err(format!("{STEP}: git tracks no file {}", path.display()));
        }
        std::fs::read(self.root.join(path))
            .map_err(|e| format!("{STEP}: reading {}: {e}", path.display()))
    }

    fn read(&self, path: &Path) -> Result<String, String> {
        String::from_utf8(self.bytes(path)?)
            .map_err(|e| format!("{STEP}: {} is not UTF-8: {e}", path.display()))
    }
}

/// Returns `dir` joined with `relative` with each `.` and `..` resolved by name, or `None` when
/// the result is absolute or leaves the checkout.
fn join(dir: &Path, relative: &str) -> Option<PathBuf> {
    let mut out = dir.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(name) => out.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// Fail when a tracked file of the checkout at `root` breaks a rule of the nested `AGENTS.md`
/// files or of `docs/SUMMARY.md`, printing each break. Reads only the files git tracks.
pub fn run(root: &Path) -> Result<(), String> {
    let problems = problems(&Tree::tracked(root)?)?;
    for problem in &problems {
        say(problem)?;
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{STEP} failed: {} problems in the tracked files",
            problems.len()
        ))
    }
}

fn problems(tree: &Tree) -> Result<Vec<String>, String> {
    let mut problems = agents::problems(tree)?;
    problems.extend(summary(tree)?);
    Ok(problems)
}

const SUMMARY: &str = "docs/SUMMARY.md";

/// Returns a line naming the pages under `docs/manual`, `docs/spec` and `docs/design` that
/// `docs/SUMMARY.md` omits, and one naming the paths it lists that are no tracked file.
fn summary(tree: &Tree) -> Result<Vec<String>, String> {
    let docs = Path::new("docs");
    let listed: BTreeSet<String> = tree
        .read(Path::new(SUMMARY))?
        .lines()
        .filter_map(|line| line.split_once("](")?.1.split_once(')'))
        .map(|(path, _)| path.to_owned())
        .collect();
    let pages: BTreeSet<String> = ["manual", "spec", "design"]
        .into_iter()
        .flat_map(|section| tree.under(&docs.join(section)))
        .filter(|path| tree.is_file(path) && path.extension().is_some_and(|e| e == "md"))
        .filter_map(|path| path.strip_prefix(docs).ok())
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let mut out = Vec::new();
    let missing: Vec<_> = pages.difference(&listed).collect();
    if !missing.is_empty() {
        out.push(format!("{SUMMARY} omits {missing:?}"));
    }
    let absent: Vec<_> = listed
        .iter()
        .filter(|path| !join(docs, path).is_some_and(|path| tree.is_file(&path)))
        .collect();
    if !absent.is_empty() {
        out.push(format!("{SUMMARY} lists {absent:?}"));
    }
    Ok(out)
}
