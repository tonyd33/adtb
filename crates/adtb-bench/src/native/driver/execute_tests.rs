use super::*;
use serde_json::Value;

struct BudgetClock;
impl Clock for BudgetClock {
    fn now(&self) -> adtb_sys::runtime::Instant {
        adtb_sys::runtime::Instant::from_nanos(0)
    }
    fn wall(&self) -> std::time::SystemTime {
        std::time::SystemTime::UNIX_EPOCH
    }
}

#[test]
fn call_spans_all_actual_attempts_and_exhaustion_has_no_next_attempt() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/reporting/producer-cases.json");
    let rows: Vec<Value> =
        serde_json::from_reader(std::fs::File::open(file).expect("read producer cases"))
            .expect("JSON cases");
    let mut reached = 0;
    for row in rows {
        let id = counter::ActorId(0);
        let mut worker = Worker {
            result: Execution {
                actor: Actor {
                    events: Vec::new(),
                    error: None,
                },
                timing: ActorTiming {
                    preparation: Span {
                        name: "prepare",
                        actor: Some(id),
                        start_ns: 0,
                        end_ns: 0,
                    },
                    calls: Vec::new(),
                    attempts: Vec::new(),
                },
            },
            id,
            runtime: BudgetClock,
            metric: MetricClock::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let mut invoked = 0;
        worker.call(
            CallId(0),
            Action::Bump(Id(1)),
            adtb_sys::runtime::Instant::from_nanos(row["budget_ns"].as_u64().expect("budget")),
            |_| {
                invoked += 1;
                if invoked <= row["retries"].as_u64().expect("retry count") {
                    Err(Failure::Retry("confirmed rollback".into()))
                } else {
                    Ok((Output::Number(1), None))
                }
            },
        );
        let calls = &worker.result.timing.calls;
        let attempts = &worker.result.timing.attempts;
        assert_eq!(calls.len(), 1, "one logical call: {row}");
        assert_eq!(u64::from(calls[0].attempts), row["attempts"], "{row}");
        assert_eq!(invoked, row["attempts"], "{row}");
        assert_eq!(attempts.len() as u64, row["attempts"], "{row}");
        if attempts.is_empty() {
            assert_eq!(
                calls[0].terminal,
                Terminal::Failed("budget_exhausted"),
                "{row}"
            );
            assert_eq!(
                worker.result.actor.error.as_deref(),
                Some("cooperative actor queue budget exhausted")
            );
        } else {
            assert!(
                calls[0].offered_ns <= attempts[0].started_ns,
                "first attempt belongs to offered call: {row}"
            );
            assert!(
                calls[0].offered_ns < attempts.last().expect("actual attempt").started_ns,
                "retries/backoff remain in call latency: {row}"
            );
            assert_eq!(
                calls[0].ended_ns,
                attempts.last().expect("actual attempt").ended_ns,
                "{row}"
            );
        }
        let actual = if calls[0].terminal == Terminal::Completed {
            "completed"
        } else {
            "failed"
        };
        assert_eq!(actual, row["outcome"], "{row}");
        assert_eq!(
            worker.result.actor.events.len(),
            attempts.len(),
            "every real attempt retained: {row}"
        );
        reached += 1;
    }
    assert_eq!(reached, 3);
}
