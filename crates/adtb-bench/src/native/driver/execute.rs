use super::*;
use reporting::{
    ActorTiming, AttemptSample, CallSample, Execution, MetricClock, RunGate, Span, Terminal,
};

struct Worker<C> {
    result: Execution,
    id: counter::ActorId,
    runtime: C,
    metric: MetricClock,
    cancel: Arc<AtomicBool>,
}

pub(super) fn execute(
    adapter: Adapter,
    queue: Vec<Action>,
    gate: Arc<RunGate>,
    cancel: Arc<AtomicBool>,
    clock: OsRuntime,
    metric: MetricClock,
    id: counter::ActorId,
) -> Execution {
    let start_ns = metric.stamp();
    let prepared = adapter.prepare();
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
                    start_ns,
                    end_ns: metric.stamp(),
                },
                calls: Vec::new(),
                attempts: Vec::new(),
            },
        },
        id,
        runtime: clock,
        metric,
        cancel,
    };
    gate.ready.wait();
    gate.wait();
    let mut prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            worker.fail(format!("prepare: {error:?}"));
            return worker.result;
        }
    };
    let deadline = worker.runtime.now() + Duration::from_secs(120);
    for (call, action) in queue.into_iter().enumerate() {
        if worker.cancel.load(Ordering::Acquire) {
            break;
        }
        worker.call(CallId(call), action, deadline, |remaining| {
            prepared.call(action, remaining)
        });
        if worker.result.actor.error.is_some() {
            break;
        }
    }
    worker.result
}

impl<C: Clock> Worker<C> {
    fn fail(&mut self, error: String) {
        self.result.actor.error = Some(error);
        self.cancel.store(true, Ordering::Release);
    }

    fn call(
        &mut self,
        call: CallId,
        action: Action,
        deadline: adtb_sys::runtime::Instant,
        mut invoke: impl FnMut(Duration) -> Result<(Output, Option<adtb::Position>), Failure>,
    ) {
        let offered_ns = self.metric.stamp();
        let mut ended_ns = offered_ns;
        let mut attempts = 0;
        let mut terminal = Terminal::Failed("budget_exhausted");
        for attempt in 1..=100 {
            if self.runtime.now() >= deadline {
                ended_ns = self.metric.stamp();
                terminal = Terminal::Failed("budget_exhausted");
                self.fail("cooperative actor queue budget exhausted".into());
                break;
            }
            let started_ns = self.metric.stamp();
            let result = invoke(deadline.saturating_since(self.runtime.now()));
            ended_ns = self.metric.stamp();
            attempts += 1;
            let retry = matches!(result, Err(Failure::Retry(_)));
            let error = match &result {
                Ok((output, _))
                    if !matches!(action, Action::Bump(_))
                        && *output != model::expected_output(action) =>
                {
                    Some(format!("output {action:?}: {output:?}"))
                }
                Err(error) if !retry => Some(format!("call {action:?}: {error:?}")),
                _ => None,
            };
            let outcome = attempt_outcome(&result);
            terminal = match &result {
                Ok(_) if error.is_some() => Terminal::Failed("output_mismatch"),
                Ok(_) => Terminal::Completed,
                Err(Failure::Ambiguous(_)) => Terminal::Uncertain,
                Err(Failure::Refused(_)) => Terminal::Failed("refused"),
                Err(Failure::Retry(_)) => Terminal::Failed("retry_exhausted"),
                Err(Failure::Fatal(_)) => Terminal::Failed("fatal"),
            };
            self.result.timing.attempts.push(AttemptSample {
                actor: self.id,
                call: CallId(call.0),
                attempt,
                started_ns,
                ended_ns,
                outcome,
            });
            self.result.actor.events.push(Event {
                call: CallId(call.0),
                action,
                attempt,
                result,
            });
            if let Some(error) = error {
                self.fail(error);
                break;
            }
            if !retry {
                break;
            }
            if attempt == 100 {
                self.fail("100 rollback-confirmed attempts exhausted".into());
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        self.result.timing.calls.push(CallSample {
            actor: self.id,
            call,
            action,
            offered_ns,
            ended_ns,
            terminal,
            attempts,
        });
    }
}

fn attempt_outcome(result: &Result<(Output, Option<adtb::Position>), Failure>) -> &'static str {
    match result {
        Ok(_) => "completed",
        Err(Failure::Retry(_)) => "retry",
        Err(Failure::Refused(_)) => "refused",
        Err(Failure::Ambiguous(_)) => "uncertain",
        Err(Failure::Fatal(_)) => "fatal",
    }
}

#[cfg(test)]
#[path = "execute_tests.rs"]
mod tests;
