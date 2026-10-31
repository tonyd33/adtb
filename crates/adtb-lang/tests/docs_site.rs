use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use adtb_lang::lex::Kw;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn highlighter_keywords() -> BTreeSet<String> {
    let path = repo().join("tools/docs-site/adt-highlight.js");
    let text = read(&path);
    let (_, rest) = text
        .split_once("keyword: '")
        .unwrap_or_else(|| panic!("{}: no keyword list", path.display()));
    let (list, _) = rest
        .split_once('\'')
        .unwrap_or_else(|| panic!("{}: unclosed keyword list", path.display()));
    list.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn highlighter_keywords_equal_the_lexer_table() {
    let lexer: BTreeSet<String> = Kw::WORDS.iter().map(|(w, _)| (*w).to_owned()).collect();
    let highlighter = highlighter_keywords();
    let missing: Vec<_> = lexer.difference(&highlighter).collect();
    let extra: Vec<_> = highlighter.difference(&lexer).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "tools/docs-site/adt-highlight.js differs from Kw::WORDS: missing {missing:?}, extra {extra:?}"
    );
}

fn summary_paths() -> BTreeSet<String> {
    read(&repo().join("docs/SUMMARY.md"))
        .lines()
        .filter_map(|line| line.split_once("](")?.1.split_once(')'))
        .map(|(path, _)| path.to_owned())
        .collect()
}

fn pages_under(dir: &Path, docs: &Path, pages: &mut BTreeSet<String>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            pages_under(&path, docs, pages);
        } else if path.extension().is_some_and(|e| e == "md") {
            let relative = path
                .strip_prefix(docs)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            pages.insert(relative.to_string_lossy().into_owned());
        }
    }
}

#[test]
fn summary_lists_every_page() {
    let docs = repo().join("docs");
    let mut pages = BTreeSet::new();
    for section in ["manual", "spec", "design"] {
        pages_under(&docs.join(section), &docs, &mut pages);
    }
    let listed = summary_paths();
    let missing: Vec<_> = pages.difference(&listed).collect();
    assert!(missing.is_empty(), "docs/SUMMARY.md omits {missing:?}");
}

#[test]
fn summary_lists_only_existing_pages() {
    let docs = repo().join("docs");
    let absent: Vec<_> = summary_paths()
        .into_iter()
        .filter(|path| !docs.join(path).is_file())
        .collect();
    assert!(absent.is_empty(), "docs/SUMMARY.md lists {absent:?}");
}
