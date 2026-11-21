//! The editor grammar agrees with adtb-lang on which texts parse, and its external scanner agrees
//! with the generated parser on the external tokens.

use std::fs;
use std::path::{Path, PathBuf};

use adtb_lang::parse::{MAX_DEPTH, Options, parse_example};
use adtb_script::conformance_scripts;
use adtb_testkit::fuzz::{case, spec_corpus};
use adtb_testkit::profile::{assert_reached, sweep};
use adtb_testkit::spec::{Block, blocks};
use tree_sitter::Parser;

const FIRST_SEED: u64 = 0x5eed_0000;
const SEEDS: u64 = 1000;

/// A refusal of adtb-lang that the grammar does not make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Known {
    /// The nesting limit counts levels, which a context free grammar cannot.
    NestingLimit,
}

impl Known {
    const ALL: [Known; 1] = [Known::NestingLimit];

    /// Returns how each diagnostic of the refusal starts.
    fn diagnostic(self) -> String {
        match self {
            Known::NestingLimit => format!("the text nests more than {MAX_DEPTH} levels deep"),
        }
    }

    /// Returns a text adtb-lang refuses for this reason alone and the grammar parses.
    fn witness(self) -> String {
        match self {
            Known::NestingLimit => {
                let depth = MAX_DEPTH as usize + 1;
                format!("{}1{}\n", "(".repeat(depth), ")".repeat(depth))
            }
        }
    }

    /// Returns the disagreement every diagnostic of a refusal is, if one is.
    fn of(diagnostics: &[String]) -> Option<Known> {
        Known::ALL.into_iter().find(|k| {
            !diagnostics.is_empty() && diagnostics.iter().all(|d| d.starts_with(&k.diagnostic()))
        })
    }
}

struct Text {
    name: String,
    text: String,
}

#[derive(Default)]
struct Tally {
    accepted: usize,
    mismatches: Vec<String>,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&adtb_tree_sitter::LANGUAGE.into())
        .expect("the generated parser's ABI is one the runtime reads");
    parser
}

/// Returns whether the grammar parses `text` without an error or a missing node.
fn grammar_accepts(parser: &mut Parser, text: &str) -> bool {
    let tree = parser.parse(text, None).expect("the parser has a language");
    !tree.root_node().has_error()
}

/// Returns adtb-lang's diagnostics of `text`, or `None` when it accepts the text.
fn adtb_refusal(text: &str) -> Option<Vec<String>> {
    let parsed = parse_example(text, Options { elisions: true });
    let diagnostics: Vec<String> = parsed.diags.iter().map(|d| d.message.clone()).collect();
    (parsed.value.is_none() || !diagnostics.is_empty()).then_some(diagnostics)
}

fn compare(texts: &[Text]) -> Tally {
    let mut parser = parser();
    let mut tally = Tally::default();
    for Text { name, text } in texts {
        let refusal = adtb_refusal(text);
        let grammar = grammar_accepts(&mut parser, text);
        let Some(diagnostics) = refusal else {
            tally.accepted += 1;
            if !grammar {
                tally
                    .mismatches
                    .push(format!("rejects what adtb-lang accepts: {name}"));
            }
            continue;
        };
        if grammar && Known::of(&diagnostics).is_none() {
            tally.mismatches.push(format!(
                "accepts what adtb-lang refuses: {name}: {}",
                diagnostics.join("; ")
            ));
        }
    }
    tally
}

/// Fail unless the grammar parses exactly the texts adtb-lang accepts, but for refusals of a
/// [`Known`] disagreement, and at least `floor` texts.
///
/// Returns how many texts adtb-lang accepts.
#[track_caller]
fn agree(part: &str, texts: &[Text], floor: usize) -> usize {
    assert_reached(&format!("{part} texts"), texts.len(), floor);
    let tally = compare(texts);
    assert!(
        tally.mismatches.is_empty(),
        "{part}: the grammar and adtb-lang disagree:\n{}",
        tally.mismatches.join("\n")
    );
    tally.accepted
}

#[test]
fn every_known_disagreement_has_a_witness() {
    let mut parser = parser();
    for known in Known::ALL {
        let witness = known.witness();
        let refusal = adtb_refusal(&witness);
        assert_eq!(
            refusal.as_deref().and_then(Known::of),
            Some(known),
            "adtb-lang's refusal of the witness of {known:?}: {refusal:?}"
        );
        assert!(
            grammar_accepts(&mut parser, &witness),
            "the grammar refuses the witness of {known:?}"
        );
    }
}

#[test]
fn the_grammar_agrees_with_adtb_lang_on_the_spec_examples() {
    let texts: Vec<Text> = blocks()
        .into_iter()
        .filter(Block::is_adt)
        .map(|b| Text {
            name: format!("docs/spec/{}:{}", b.file, b.line),
            text: b.text,
        })
        .collect();
    agree("spec", &texts, 300);
}

/// Returns each entry's name and source in a tree-sitter corpus file.
fn corpus_sources(text: &str) -> Vec<(String, String)> {
    let mut sources = Vec::new();
    for entry in text.split("\n---\n") {
        let lines: Vec<&str> = entry.lines().collect();
        let Some(rule) = lines.iter().rposition(|l| l.starts_with("====")) else {
            continue;
        };
        let title = lines[..rule]
            .iter()
            .rev()
            .find(|l| !l.starts_with("====") && !l.trim().is_empty())
            .copied()
            .unwrap_or_default();
        sources.push((title.to_owned(), lines[rule + 1..].join("\n") + "\n"));
    }
    sources
}

#[test]
fn the_grammar_agrees_with_adtb_lang_on_its_corpus() {
    let dir = root().join("tools/tree-sitter-adtb/test/corpus");
    let mut texts = Vec::new();
    for path in files(&dir) {
        for (title, text) in corpus_sources(&read(&path)) {
            texts.push(Text {
                name: format!("{}: {title}", path.display()),
                text,
            });
        }
    }
    agree("corpus", &texts, 40);
}

/// Returns the files under `dir`, outside any `target` directory, sorted.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if !path.is_dir() {
                out.push(path);
            } else if path.file_name().is_some_and(|n| n != "target") {
                pending.push(path);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn the_grammar_agrees_with_adtb_lang_on_the_conformance_scripts() {
    let scripts =
        conformance_scripts(&root().join("conformance")).unwrap_or_else(|e| panic!("{e}"));
    let mut texts = Vec::new();
    for (name, script) in scripts {
        for d in script.directives() {
            texts.push(Text {
                name: format!("{name}:{}", d.line()),
                text: d.body().to_owned(),
            });
        }
    }
    agree("conformance", &texts, 2500);
}

#[test]
fn the_grammar_agrees_with_adtb_lang_on_fuzzed_texts() {
    let corpus = spec_corpus(&root().join("docs/spec"));
    let texts: Vec<Text> = sweep(FIRST_SEED..FIRST_SEED + SEEDS)
        .map(|seed| Text {
            name: format!("fuzz seed {seed}"),
            text: case(seed, &corpus).text,
        })
        .collect();
    let accepted = agree("fuzz", &texts, 0);
    assert_reached("fuzz texts adtb-lang accepts", accepted, texts.len() / 4);
}

#[test]
fn the_scanner_declares_the_externals_of_the_grammar_in_order() {
    let src = root().join("tools/tree-sitter-adtb/src");
    let grammar: serde_json::Value =
        serde_json::from_str(&read(&src.join("grammar.json"))).expect("grammar.json is JSON");
    let externals: Vec<String> = grammar["externals"]
        .as_array()
        .expect("grammar.json lists its externals")
        .iter()
        .map(|e| {
            let name = e["name"].as_str().expect("an external is a named symbol");
            name.strip_prefix('_').unwrap_or(name).to_uppercase()
        })
        .collect();
    let scanner = read(&src.join("scanner.c"));
    let (symbols, _) = scanner
        .split_once("enum Symbol {")
        .and_then(|(_, rest)| rest.split_once("};"))
        .expect("scanner.c declares enum Symbol");
    let symbols: Vec<&str> = symbols
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(symbols, externals);
}

#[test]
fn the_highlight_tests_parse() {
    let texts = files(&root().join("tools/tree-sitter-adtb/test/highlight"));
    assert_reached("highlight tests", texts.len(), 1);
    let mut parser = parser();
    for path in texts {
        assert!(
            grammar_accepts(&mut parser, &read(&path)),
            "the grammar refuses {}",
            path.display()
        );
    }
}
