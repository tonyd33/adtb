use std::path::Path;

fn main() {
    let src = Path::new("../../tools/tree-sitter-adtb/src");
    for file in ["parser.c", "scanner.c", "tree_sitter"] {
        println!("cargo:rerun-if-changed={}", src.join(file).display());
    }
    cc::Build::new()
        .std("c11")
        .include(src)
        .warnings(false)
        .file(src.join("parser.c"))
        .compile("tree-sitter-adtb-parser");
    cc::Build::new()
        .std("c11")
        .include(src)
        .opt_level(2)
        .warnings_into_errors(true)
        .file(src.join("scanner.c"))
        .compile("tree-sitter-adtb-scanner");
}
