use std::fs;
use std::path::Path;

use adtdb_lang::parse::{Options, parse_example};

struct Block {
    file: String,
    line: usize,
    info: String,
    text: String,
}

fn blocks() -> Vec<Block> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec");
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let src = fs::read_to_string(&path).unwrap();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let mut current: Option<Block> = None;
        for (i, line) in src.lines().enumerate() {
            let trimmed = line.trim_start();
            if let Some(info) = trimmed.strip_prefix("```") {
                match current.take() {
                    Some(b) => out.push(b),
                    None => {
                        current = Some(Block {
                            file: file.clone(),
                            line: i + 1,
                            info: info.trim().to_string(),
                            text: String::new(),
                        })
                    }
                }
            } else if let Some(b) = current.as_mut() {
                b.text.push_str(line);
                b.text.push('\n');
            }
        }
    }
    out
}

#[test]
fn spec_examples_parse() {
    let opts = Options { elisions: true };
    let mut failures = Vec::new();
    let mut checked = 0;
    for b in blocks().iter().filter(|b| b.info == "adt") {
        checked += 1;
        let parsed = parse_example(&b.text, opts);
        if !parsed.diags.is_empty() {
            let msgs: Vec<String> = parsed.diags.iter().map(|d| d.render(&b.text)).collect();
            failures.push(format!("{}:{}: {}", b.file, b.line, msgs.join("; ")));
        }
    }
    assert!(checked > 0, "no `adt` blocks found");
    assert!(
        failures.is_empty(),
        "{} of {checked} examples fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_spec_block_names_its_kind() {
    let kinds = ["adt", "ebnf", "text", "sh", "rust", "toml"];
    let bad: Vec<String> = blocks()
        .iter()
        .filter(|b| !kinds.contains(&b.info.as_str()))
        .map(|b| format!("{}:{} `{}`", b.file, b.line, b.info))
        .collect();
    assert!(
        bad.is_empty(),
        "blocks without a known kind:\n{}",
        bad.join("\n")
    );
}
