use super::*;
use std::collections::BTreeSet;
use std::sync::atomic::AtomicU64;

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn lifecycle_outputs_refusals_and_reopen() {
    let input =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/smoke.txt"))
            .expect("read runtime smoke fixture");
    let n = input.trim().parse::<i64>().expect("fixture is an integer");
    assert_eq!(n, 4, "the smoke covers all four campaigns");
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sqlite-baseline/tests");
    std::fs::create_dir_all(&scratch).expect("create test scratch");
    for peer in [Peer::Adtb, Peer::Sqlite] {
        let root = scratch.join(format!(
            "{:?}-{}-{}",
            peer,
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).expect("create a fresh database directory");
        let fixture = model::fixture(n, true);
        let mut adapter = Adapter::open(peer, &root, 1, true).expect("open real adapter");
        adapter.seed(&fixture).expect("load fixed fixture");
        assert_eq!(
            adapter.snapshot().expect("read fixture"),
            fixture,
            "{peer:?}"
        );
        let before = adapter.snapshot().expect("state before refusal");
        {
            let mut prepared = adapter.prepare().expect("prepare native statements");
            assert_eq!(
                prepared.call(Action::Finish(Id(1)), Duration::from_secs(10)),
                Err(Failure::Refused("Session is not live".into())),
                "{peer:?}"
            );
        }
        assert_eq!(
            adapter.snapshot().expect("state after refusal"),
            before,
            "{peer:?}"
        );
        for id in model::permutation(n) {
            let mut prepared = adapter.prepare().expect("prepare native statements");
            for action in [
                Action::Start(id),
                Action::ReadLive(id),
                Action::Finish(id),
                Action::ReadPlayed(id),
            ] {
                let (output, _) = prepared
                    .call(action, Duration::from_secs(10))
                    .expect("lifecycle call acknowledges");
                assert_eq!(
                    output,
                    model::expected_output(action),
                    "{peer:?} {action:?}"
                );
            }
            assert_eq!(
                prepared.call(Action::Start(id), Duration::from_secs(10)),
                Err(Failure::Refused("Session is not planned".into())),
                "{peer:?}"
            );
            assert_eq!(
                prepared.call(Action::Finish(id), Duration::from_secs(10)),
                Err(Failure::Refused("Session is not live".into())),
                "{peer:?}"
            );
        }
        let mut expected = fixture;
        expected.sessions = (1..=n)
            .map(|id| model::session(Id(id), Prefix::ReadPlayed))
            .collect();
        assert_eq!(
            adapter.snapshot().expect("whole final state"),
            expected,
            "{peer:?}"
        );
        adapter.close().expect("close durable native database");
        let reopened = Adapter::open(peer, &root, 1, false).expect("reopen same file");
        assert_eq!(
            reopened.snapshot().expect("whole reopened state"),
            expected,
            "{peer:?}"
        );
        reopened.close().expect("close reopened database");
    }
}

#[test]
fn control_requests_rotate_keys_across_actors() {
    let text =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/control.txt"))
            .expect("read runtime control cases");
    let mut cases = 0;
    for line in text.lines() {
        let values: Vec<usize> = line
            .split_whitespace()
            .map(|v| v.parse().expect("integer fixture field"))
            .collect();
        let [n, requests, writers, per_actor, visits, owners] = values.as_slice() else {
            panic!("control cases have six fields");
        };
        let queues = actions(
            i64::try_from(*n).expect("fixture N fits"),
            *writers,
            false,
            *requests,
        );
        assert_eq!(queues.len(), *writers);
        let mut seen: BTreeMap<Id, (usize, BTreeSet<usize>)> = BTreeMap::new();
        for (actor, queue) in queues.iter().enumerate() {
            assert_eq!(queue.len(), *per_actor, "case {line}");
            for action in queue {
                let Action::Bump(id) = action else {
                    panic!("counter queue contains only increments");
                };
                let (count, actors) = seen.entry(*id).or_default();
                *count += 1;
                actors.insert(actor);
            }
        }
        assert_eq!(
            seen.keys().copied().collect::<Vec<_>>(),
            (1..=i64::try_from(*n).expect("fixture N fits"))
                .map(Id)
                .collect::<Vec<_>>()
        );
        for (id, (count, actors)) in seen {
            assert_eq!(
                (count, actors.len()),
                (*visits, *owners),
                "case {line}, {id:?}"
            );
        }
        cases += 1;
    }
    assert_eq!(cases, 3, "smoke and both planned datasets ran");
}

#[test]
fn counter_audit_requires_compatible_history() {
    let input = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/history-cases.txt"),
    )
    .expect("read runtime counter histories");
    let mut checked = 0;
    for line in input.lines() {
        let fields: Vec<_> = line.split('|').collect();
        let [name, operations, final_values, expected] = fields.as_slice() else {
            panic!("four history fields");
        };
        let histories = super::super::history_fixture::history(operations);
        let actors: Vec<_> = histories
            .iter()
            .map(|history| Actor {
                events: history
                    .iter()
                    .enumerate()
                    .map(|(call, observation)| Event {
                        call: CallId(call),
                        action: Action::Bump(observation.key),
                        attempt: 1,
                        result: match observation.outcome {
                            counter::Outcome::Acknowledged(value) => {
                                Ok((Output::Number(value), None))
                            }
                            counter::Outcome::Uncertain => Err(Failure::Ambiguous(
                                "identified in-flight fixture call".into(),
                            )),
                        },
                    })
                    .collect(),
                error: None,
            })
            .collect();
        let final_values = super::super::history_fixture::finals(final_values);
        let fixture = model::fixture(
            i64::try_from(final_values.len()).expect("fixture size fits"),
            false,
        );
        let mut actual = fixture.clone();
        for c in &mut actual.counters {
            c.value = final_values[&Id(c.id)];
        }
        let expected = match *expected {
            "compatible" => Ok(()),
            "incompatible" => Err("counter history: Incompatible".to_owned()),
            other => panic!("unknown fixture outcome {other}"),
        };
        assert_eq!(
            audit(&fixture, &actual, &actors, false).map(|_| ()),
            expected,
            "{name}"
        );
        checked += 1;
    }
    assert_eq!(
        checked, 9,
        "all order and uncertainty cases reach the live audit gate"
    );
}
