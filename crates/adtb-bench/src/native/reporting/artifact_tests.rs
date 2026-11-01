use super::*;

#[test]
fn finish_alone_is_partial_and_read_played_closes_workflow_span() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/reporting/producer-workflows.json");
    let rows: Vec<Value> = serde_json::from_reader(File::open(path).expect("read workflow cases"))
        .expect("JSON cases");
    let mut reached = 0;
    for row in rows {
        let actions = [
            Action::Start(Id(1)),
            Action::ReadLive(Id(1)),
            Action::Finish(Id(1)),
            Action::ReadPlayed(Id(1)),
        ];
        let calls: Vec<_> = actions
            .into_iter()
            .take(usize::try_from(row["calls"].as_u64().expect("call count")).expect("small case"))
            .enumerate()
            .map(|(i, action)| CallSample {
                actor: counter::ActorId(0),
                call: CallId(i),
                action,
                offered_ns: 1 + i as u64 * 10,
                ended_ns: 2 + i as u64 * 10,
                terminal: Terminal::Completed,
                attempts: 1,
            })
            .collect();
        let refs: Vec<_> = calls.iter().collect();
        let workflows = workflow_rows(&refs);
        assert_eq!(
            workflows,
            [
                json!({"actor":0,"workflow":1,"offered_ns":1,"ended_ns":row["end_ns"],"outcome":row["outcome"],"calls":row["calls"]})
            ],
            "{row}"
        );
        reached += 1;
    }
    assert_eq!(reached, 2);
}
