//! Writes the files and the Rust examples of the manual's pages under `OUT_DIR`, one directory
//! per page laid out as the reader's crate, and runs the Rust API page's build script there.
//!
//! A fenced block whose info string is a language and a path, such as `adt migrations/0001.adt`,
//! is a file of the reader's crate. A block whose info string is `rust` is an example: a program
//! whose `main` a test runs, with what it prints compared to the `text` block right after it,
//! or to nothing when no `text` block follows.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

/// The pages with Rust examples, under `docs/manual`.
const PAGES: [&str; 2] = ["reference/rust-api.md", "reference/errors.md"];

/// The page whose `rust build.rs` block is the build script [`page::run`] runs.
const BUILT: &str = "reference/rust-api.md";

const PAGE_BUILD: &str = include_str!("page/build.rs");

struct Example {
    line: usize,
    code: String,
    shown: String,
}

/// Returns the files of `page` by path, and its examples.
fn blocks(page: &str) -> (Vec<(String, String)>, Vec<Example>) {
    let mut files = Vec::new();
    let mut examples: Vec<Example> = Vec::new();
    let mut lines = page.lines().enumerate();
    let mut after_example = false;
    while let Some((n, line)) = lines.next() {
        let Some(info) = line.strip_prefix("```") else {
            after_example &= line.trim().is_empty();
            continue;
        };
        let body: String = lines
            .by_ref()
            .map(|(_, l)| l)
            .take_while(|l| !l.starts_with("```"))
            .fold(String::new(), |s, l| s + l + "\n");
        let follows = std::mem::take(&mut after_example);
        match info.split_once(' ') {
            Some((_, name)) if name != "fragment" => files.push((name.to_owned(), body)),
            None if info == "rust" => {
                examples.push(Example {
                    line: n + 1,
                    code: body,
                    shown: String::new(),
                });
                after_example = true;
            }
            None if info == "text" && follows => {
                examples.last_mut().expect("an example").shown = body;
            }
            _ => {}
        }
    }
    (files, examples)
}

/// Returns the test module of `examples` of `page`: one module per example holding its code, a
/// `println!` that records what it prints, and a test that runs its `main`.
fn tests(page: &str, examples: &[Example]) -> String {
    let mut out = String::new();
    for e in examples {
        let line = e.line;
        writeln!(
            out,
            "mod line_{line} {{\n\
             macro_rules! println {{\n\
             () => {{ ::adtb_manual::print(String::new()) }};\n\
             ($($t:tt)*) => {{ ::adtb_manual::print(format!($($t)*)) }};\n\
             }}\n\
             {code}\n\
             #[test]\n\
             fn runs() {{\n\
             let dir = std::path::Path::new(env!(\"CARGO_TARGET_TMPDIR\"))\n\
             .join(\"rust-examples\").join({page:?}).join(\"line-{line}\");\n\
             ::adtb_manual::run({page:?}, {line}, &dir, {shown:?}, main);\n\
             }}\n\
             }}",
            code = e.code,
            shown = e.shown,
        )
        .expect("a String takes writes");
    }
    out
}

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    }
    std::fs::write(path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// The build script of page [`BUILT`].
mod page {
    include!("page/build.rs");

    pub(super) fn run() {
        main();
    }
}

fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let mut directives = std::io::stdout();
    writeln!(directives, "cargo::rerun-if-changed=page/build.rs").expect("cargo reads directives");
    for page in PAGES {
        let path = format!("../../docs/manual/{page}");
        writeln!(directives, "cargo::rerun-if-changed={path}").expect("cargo reads directives");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let (files, examples) = blocks(&text);
        let dir = out.join(page);
        for (name, body) in &files {
            if page == BUILT && name == "build.rs" {
                assert_eq!(
                    PAGE_BUILD, body,
                    "page/build.rs is the build script of {page}"
                );
            }
            write(&dir.join(name), body);
        }
        write(&dir.join("src/examples.rs"), &tests(page, &examples));
    }
    std::env::set_current_dir(out.join(BUILT)).expect("the page's crate directory");
    page::run();
}
