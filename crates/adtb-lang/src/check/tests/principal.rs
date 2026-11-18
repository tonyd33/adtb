use super::*;

#[test]
fn root_principal_checks_in_unrelated_schema_functions_and_bundle_scopes() {
    let files = [
        "module identity\ntype Principal = EndUser { level: I8 } | Service { name: Text } | Anonymous",
        "module consumer\nfn me () -> Principal = caller\nfn end_user () -> Principal = Principal.EndUser { level: 127 }",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, text)| (FileId(i as u32), parse_schema(text, Options::default()).ok().unwrap()))
    .collect::<Vec<_>>();
    let schema = build(&files).unwrap();
    let program = check_schema(&schema).unwrap();
    let scopes = [
        Visible::of(Vec::new()),
        Visible::module(schema.module_named("consumer").unwrap()),
        Visible::module(schema.module_named("identity").unwrap()),
        Visible::all(&schema),
    ];
    for scope in scopes {
        for text in [
            "query me () -> Principal = caller",
            "query anonymous () -> Principal = Anonymous",
            "query end_user () -> Principal = Principal.EndUser { level: 127 }",
            "query level () -> I8 = match caller | Principal.EndUser e -> e.level | Service _ -> 0 | Anonymous -> -1",
        ] {
            let mut items = parse_example(text, Options::default()).ok().unwrap();
            let ExampleItem::BundleItem(BundleItem::Query(query)) = items.remove(0) else {
                panic!("the example holds a query");
            };
            check_query_in(&schema, &program, scope.clone(), &query)
                .unwrap_or_else(|e| panic!("{scope:?}: {text}: {e:?}"));
        }
        let incomplete =
            "query incomplete () -> I8 = match caller | EndUser e -> e.level | Service _ -> 0";
        let mut items = parse_example(incomplete, Options::default()).ok().unwrap();
        let ExampleItem::BundleItem(BundleItem::Query(query)) = items.remove(0) else {
            panic!("the example holds a query");
        };
        let at = incomplete.find("match").unwrap();
        assert_eq!(
            Err(Diag::new(
                Span::new(at, incomplete.len()),
                "match does not cover `Anonymous`"
            )),
            check_query_in(&schema, &program, scope.clone(), &query).map(|_| ()),
            "{scope:?}"
        );
    }
}
