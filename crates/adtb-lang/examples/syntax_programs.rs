//! Write the texts the tree-sitter grammar is checked against into a directory, one file each, and
//! a manifest `verdict<TAB>name<TAB>diagnostic` per line. `verdict` is `accept` or `refuse` by
//! `parse_example`, the language the grammar's root states.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use adtb_lang::parse::{Options, parse_example};
use adtb_testkit::fuzz::{case, spec_corpus};
use adtb_testkit::spec::blocks;

const FIRST_SEED: u64 = 0x5eed_0000;
const CASES: u64 = 4000;

struct Out {
    dir: PathBuf,
    manifest: String,
}

impl Out {
    fn add(&mut self, name: &str, text: &str) {
        let parsed = parse_example(text, Options { elisions: true });
        let refused = parsed.value.is_none() || !parsed.diags.is_empty();
        let diag = parsed
            .diags
            .iter()
            .map(|d| d.message.replace(['\n', '\t'], " "))
            .collect::<Vec<_>>()
            .join("; ");
        let verdict = if refused { "refuse" } else { "accept" };
        let file = format!("{name}.adt");
        let path = self.dir.join(&file);
        fs::create_dir_all(path.parent().expect("a file has a parent"))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        fs::write(&path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        writeln!(self.manifest, "{verdict}\t{file}\t{diag}").expect("a string takes a line");
    }
}

/// Returns the bodies of a conformance script's directives, skipping the rows of a query.
fn script_bodies(text: &str) -> Vec<String> {
    let mut bodies = Vec::new();
    let mut body: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("  ") {
            if let Some(b) = &mut body {
                b.push_str(rest);
                b.push('\n');
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        bodies.extend(body.take());
        let head = line.split_whitespace().next().unwrap_or_default();
        if matches!(head, "schema" | "run" | "query" | "error") {
            body = Some(String::new());
        }
    }
    bodies.extend(body);
    bodies
}

/// Returns the source of each entry of a tree-sitter corpus file.
fn corpus_sources(text: &str) -> Vec<String> {
    let mut sources = Vec::new();
    for entry in text.split("\n---\n") {
        let lines: Vec<&str> = entry.lines().collect();
        let Some(rule) = lines.iter().rposition(|l| l.starts_with("====")) else {
            continue;
        };
        sources.push(lines[rule + 1..].join("\n") + "\n");
    }
    sources
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "target") {
                files(&path, out);
            }
        } else {
            out.push(path);
        }
    }
    out.sort();
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("an output directory"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Out {
        dir: dir.clone(),
        manifest: String::new(),
    };
    for (i, b) in blocks().iter().filter(|b| b.is_adt()).enumerate() {
        out.add(&format!("spec/{i:04}"), &b.text);
    }
    let mut paths = Vec::new();
    files(&root.join("apps/campaign"), &mut paths);
    for (i, p) in paths
        .iter()
        .filter(|p| p.extension().is_some_and(|x| x == "adt" || x == "adb"))
        .enumerate()
    {
        let text = fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        out.add(&format!("campaign/{i:04}"), &text);
    }
    let mut paths = Vec::new();
    files(&root.join("tools/tree-sitter-adtb/test/corpus"), &mut paths);
    let mut n = 0;
    for p in &paths {
        let text = fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        for source in corpus_sources(&text) {
            n += 1;
            out.add(&format!("corpus/{n:04}"), &source);
        }
    }
    let mut paths = Vec::new();
    files(&root.join("conformance"), &mut paths);
    let mut n = 0;
    for p in paths.iter().filter(|p| {
        p.to_str()
            .is_some_and(|s| s.ends_with(".test") || s.ends_with(".test.in"))
    }) {
        let text = fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        for body in script_bodies(&text) {
            n += 1;
            out.add(&format!("conformance/{n:05}"), &body);
        }
    }
    let corpus = spec_corpus(&root.join("docs/spec"));
    for seed in FIRST_SEED..FIRST_SEED + CASES {
        out.add(&format!("fuzz/{seed}"), &case(seed, &corpus).text);
    }
    let manifest = dir.join("manifest");
    fs::write(&manifest, &out.manifest).unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
}
