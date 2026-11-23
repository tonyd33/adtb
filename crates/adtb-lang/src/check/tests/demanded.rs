use super::*;
use crate::ast::SchemaFile;

/// A body that fails is reported once, under its own file and at its own span, whichever body in
/// whichever file reads it.
#[test]
fn a_failing_computed_field_is_reported_once_in_its_own_file_whatever_reads_it() {
    let declaring = "module app\ntype T { id: Key ts } = A { a: I64 } | B\nrelation ts : T\n  \
                     key id generate sequence\n  computed zz: I64 = \"a\"\n";
    let readers = [
        "constraint big = from t in ts where t.zz > 3 select t",
        "check small on ts (zz < 9)",
        "check small_a on ts.A (zz < 9)",
        "fn total () -> I64 = sum(t.zz for t in ts)",
        "fn of_a () -> Set I64 = from t in ts.A select t.zz",
    ];
    let at = declaring.find("\"a\"").expect("the failing text");
    let want = vec![(
        FileId(0),
        Span::new(at, at + 3),
        "expected I64, found Text".to_string(),
    )];
    let mut ran = 0;
    for reader in readers {
        for apart in [false, true] {
            let files: Vec<String> = if apart {
                vec![declaring.to_string(), format!("module app\n{reader}\n")]
            } else {
                vec![format!("{declaring}{reader}\n")]
            };
            let parsed: Vec<(FileId, SchemaFile)> = files
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let file = parse_schema(f, Options::default()).ok().expect("parses");
                    (FileId(i as u32), file)
                })
                .collect();
            let s = build(&parsed).expect("the schema builds");
            let got: Vec<(FileId, Span, String)> = match check_schema(&s) {
                Ok(_) => Vec::new(),
                Err(ds) => ds
                    .into_iter()
                    .map(|d| (d.file, d.diag.span, d.diag.message))
                    .collect(),
            };
            assert_eq!(want, got, "{reader}, in a file apart: {apart}");
            ran += 1;
        }
    }
    assert_eq!(readers.len() * 2, ran);
}
