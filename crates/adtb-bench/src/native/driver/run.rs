use super::*;
use reporting::{Config, Report, RunGate};

pub fn correctness(
    peer: Peer,
    root: PathBuf,
    n: i64,
    writers: usize,
    lifecycle: bool,
    requests: usize,
) -> Result<String, String> {
    if ![1, 4].contains(&writers) || !(n == 1000 || n == 10000 || n == 4) {
        return Err("N must be 4 (smoke),1000,10000; writers 1 or 4".into());
    }
    let mut report = Report::new(Config {
        peer,
        n,
        writers,
        lifecycle,
        requests,
    });
    report.measure("directory_create", || {
        std::fs::create_dir(&root).map_err(|e| e.to_string())
    })?;
    let outcome = run(&root, &mut report);
    let written = report.finish(&root, &outcome);
    match (outcome, written) {
        (outcome, Ok(())) => outcome,
        (Ok(_), Err(error)) => Err(format!("report artifacts: {error}")),
        (Err(error), Err(report)) => Err(format!("{error}; report artifacts: {report}")),
    }
}

fn failure(error: Failure) -> String {
    format!("{error:?}")
}

fn run(root: &Path, report: &mut Report) -> Result<String, String> {
    let Config {
        peer,
        n,
        writers,
        lifecycle,
        requests,
    } = report.config;
    let (fixture, queues) = report.measure("fixture_generation", || {
        Ok((
            model::fixture(n, lifecycle),
            actions(n, writers, lifecycle, requests),
        ))
    })?;
    for (actor, queue) in queues.iter().enumerate() {
        report.plan.extend(
            queue
                .iter()
                .enumerate()
                .map(|(call, action)| (counter::ActorId(actor), CallId(call), *action)),
        );
    }
    let mut adapter = report.measure("initial_open", || {
        Adapter::open(peer, root, writers, true).map_err(failure)
    })?;
    report.settings = report.measure("settings", || adapter.settings().map_err(failure))?;
    report.measure("load", || adapter.seed(&fixture).map_err(failure))?;
    report.measure("loaded_validation", || {
        if adapter.snapshot().map_err(failure)? != fixture {
            return Err("loaded fixture differs from independent oracle".into());
        }
        Ok(())
    })?;
    report.validation[0] = true;
    let (clock, adapters) = report.measure("actor_open", || {
        let clock = match &adapter {
            Adapter::Adtb(a) => a.clock.clone(),
            Adapter::Sqlite(_) => {
                OsRuntime::try_new(root, Backend::Pool { threads: 1 }).map_err(|e| e.to_string())?
            }
        };
        let adapters = (0..writers)
            .map(|_| adapter.actor(root, writers))
            .collect::<Result<Vec<_>, _>>()
            .map_err(failure)?;
        Ok((clock, adapters))
    })?;
    let coordinator = if peer == Peer::Sqlite {
        report.measure("coordinator_close", || adapter.close().map_err(failure))?;
        None
    } else {
        Some(adapter)
    };
    let actors = execute_actors(adapters, queues, clock, report)?;
    let adapter = match coordinator {
        Some(adapter) => adapter,
        None => report.measure("post_open", || {
            Adapter::open(peer, root, writers, false).map_err(failure)
        })?,
    };
    let settings = report.settings.clone();
    report.measure("ledger", || write_ledger(root, &settings, &actors))?;
    report.measure("validation", || {
        audit(
            &fixture,
            &adapter.snapshot().map_err(failure)?,
            &actors,
            lifecycle,
        )
        .map(|_| ())
    })?;
    report.validation[1] = true;
    report.inventory(root, true)?;
    report.measure("close", || adapter.close().map_err(failure))?;
    let reopened = report.measure("reopen", || {
        Adapter::open(peer, root, writers, false).map_err(failure)
    })?;
    let (completed, final_state) = report.measure("reopened_validation", || {
        let completed = audit(
            &fixture,
            &reopened.snapshot().map_err(failure)?,
            &actors,
            lifecycle,
        )?;
        Ok((completed, reopened.snapshot().map_err(failure)?))
    })?;
    report.validation[2] = true;
    report.measure("final_close", || reopened.close().map_err(failure))?;
    report.inventory(root, false)?;
    verify_completion(&actors, &final_state, completed, report.config)?;
    report.validation[3] = true;
    Ok(format!(
        "correctness complete: {peer:?} N={n} writers={writers} jobs={completed}; full-state and reopen passed; application reporting artifacts ready; {}",
        report.settings
    ))
}

fn execute_actors(
    adapters: Vec<Adapter>,
    queues: Vec<Vec<Action>>,
    clock: OsRuntime,
    report: &mut Report,
) -> Result<Vec<Actor>, String> {
    let expected_queues = queues.clone();
    let gate = Arc::new(RunGate::new(report.config.writers));
    let cancel = Arc::new(AtomicBool::new(false));
    let metric = report.clock;
    let (executions, released) = report.measure("actor_run", || {
        std::thread::scope(|scope| {
            let joins: Vec<_> = adapters
                .into_iter()
                .zip(queues)
                .enumerate()
                .map(|(id, (adapter, queue))| {
                    let gate = gate.clone();
                    let cancel = cancel.clone();
                    let clock = clock.clone();
                    scope.spawn(move || {
                        execute::execute(
                            adapter,
                            queue,
                            gate,
                            cancel,
                            clock,
                            metric,
                            counter::ActorId(id),
                        )
                    })
                })
                .collect();
            gate.ready.wait();
            let released = gate.release(metric);
            let executions = joins
                .into_iter()
                .map(|j| j.join().map_err(|_| "actor panic".to_owned()))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((executions, released))
        })
    })?;
    report.released = Some(released);
    let mut actors = Vec::new();
    for execution in executions {
        actors.push(execution.actor);
        report.actors.push(execution.timing);
    }
    report.incomplete = actors.iter().any(|actor| actor.error.is_some());
    for (actor, queue) in actors.iter().zip(&expected_queues) {
        for event in &actor.events {
            if queue.get(event.call.0) != Some(&event.action) {
                return Err("ledger call identity differs from frozen request list".into());
            }
        }
    }
    Ok(actors)
}

fn write_ledger(root: &Path, settings: &str, actors: &[Actor]) -> Result<(), String> {
    let mut ledger = std::fs::File::create(root.join("ledger.txt")).map_err(|e| e.to_string())?;
    writeln!(ledger, "{settings}").map_err(|e| e.to_string())?;
    for (actor, a) in actors.iter().enumerate() {
        for e in &a.events {
            writeln!(
                ledger,
                "actor={actor} call={} action={:?} attempt={} result={:?}",
                e.call.0, e.action, e.attempt, e.result
            )
            .map_err(|e| e.to_string())?;
        }
    }
    ledger.sync_all().map_err(|e| e.to_string())
}

fn verify_completion(
    actors: &[Actor],
    state: &Snapshot,
    completed: usize,
    config: Config,
) -> Result<(), String> {
    if actors.iter().any(|a| a.error.is_some()) {
        return Err(format!(
            "incomplete/invalid; retained acknowledged-prefix oracle passed; actor errors: {:?}",
            actors.iter().map(|a| &a.error).collect::<Vec<_>>()
        ));
    }
    let required = if config.lifecycle {
        usize::try_from(config.n).map_err(|e| e.to_string())?
    } else {
        config.requests
    };
    if completed != required {
        return Err(format!("incomplete: {completed}/{required} jobs"));
    }
    let per_key = i64::try_from(config.requests).map_err(|e| e.to_string())? / config.n;
    if !config.lifecycle && state.counters.iter().any(|c| c.value != per_key) {
        return Err(
            "complete control differs from closed-form requests/N increments per key".into(),
        );
    }
    Ok(())
}
