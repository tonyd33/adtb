use std::path::{Path, PathBuf};

fn part(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance")
        .join(name)
}

fn passes(name: &str) {
    let dir = part(name);
    assert!(
        !adtb_conformance::scripts(&dir).is_empty(),
        "no scripts in {}",
        dir.display()
    );
    let found = adtb_conformance::run_dir(&dir);
    assert!(
        found.is_empty(),
        "{} directives differ:\n{}",
        found.len(),
        found.join("\n")
    );
}

#[test]
fn model() {
    passes("model");
}

#[test]
fn queries() {
    passes("queries");
}

#[test]
fn writes() {
    passes("writes");
}

#[test]
fn constraints() {
    passes("constraints");
}

#[test]
fn conformance() {
    passes("conformance");
}

#[test]
fn interop() {
    passes("interop");
}

fn expand(body: &str, schema: &str, run: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        match line {
            "@SCHEMA@" => out.push_str(schema),
            "@RUN@" => out.push_str(run),
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

#[test]
fn generated_queries_match_their_bodies() {
    let gen_dir = part("gen/queries");
    let read = |path: &Path| {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    };
    let schema = read(&gen_dir.join("schema.txt"));
    let run = read(&gen_dir.join("run.txt"));
    let mut bodies: Vec<PathBuf> = std::fs::read_dir(gen_dir.join("bodies"))
        .expect("the bodies directory lists")
        .map(|entry| entry.expect("a bodies entry reads").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "in"))
        .collect();
    bodies.sort();
    assert!(!bodies.is_empty(), "no bodies in {}", gen_dir.display());
    let drifted: Vec<String> = bodies
        .iter()
        .filter_map(|body| {
            let name = body.file_stem().expect("a body has a name");
            let generated = part("queries").join(name);
            (read(&generated) != expand(&read(body), &schema, &run))
                .then(|| generated.display().to_string())
        })
        .collect();
    assert!(
        drifted.is_empty(),
        "generated files differ from their bodies; rerun conformance/gen/queries/build.sh:\n{}",
        drifted.join("\n")
    );
}
