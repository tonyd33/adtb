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
