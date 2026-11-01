use super::*;
use std::fs::File;

include!(concat!(env!("OUT_DIR"), "/build-info.rs"));

impl Report {
    pub fn finish(&mut self, root: &Path, outcome: &Result<String, String>) -> Result<(), String> {
        let process_io = storage::process_io();
        let calls: Vec<_> = self.actors.iter().flat_map(|a| &a.calls).collect();
        let attempts: Vec<_> = self.actors.iter().flat_map(|a| &a.attempts).collect();
        let workflows = workflow_rows(&calls);
        let window = self.released.zip(calls.iter().map(|c| c.ended_ns).max());
        if let Some((start_ns, end_ns)) = window {
            self.phases.push(Span {
                name: "steady",
                actor: None,
                start_ns,
                end_ns,
            });
        }
        let write_start = self.clock.stamp();
        write_rows(root, "calls.ndjson", calls.iter().map(|c| call_json(c)))?;
        write_rows(
            root,
            "attempts.ndjson",
            attempts.iter().map(|a| attempt_json(a)),
        )?;
        write_rows(root, "workflows.ndjson", workflows.iter().cloned())?;
        self.phases.push(Span {
            name: "artifact_write",
            actor: None,
            start_ns: write_start,
            end_ns: self.clock.stamp(),
        });
        let status = if outcome.is_ok() {
            "complete"
        } else if self.incomplete {
            "incomplete"
        } else {
            "invalid"
        };
        let config = self.config;
        let plan: Vec<_> = self
            .plan
            .iter()
            .map(|(actor, call, action)| {
                json!({
                    "actor":actor.0,"call":call.0,"action":action_name(*action),
                    "target":target(*action).0,"workflow":workflow(*action)
                })
            })
            .collect();
        let phases: Vec<_> = self
            .phases
            .iter()
            .chain(self.actors.iter().map(|a| &a.preparation))
            .map(span_json)
            .collect();
        let build: BTreeMap<_, _> = BUILD.iter().copied().collect();
        let value = json!({
            "schema_version":1,"kind":"adtb-bench-application","status":status,
            "error":outcome.as_ref().err(),"invocation_id":std::env::var("INVOCATION_ID").ok(),
            "peer":if config.peer == Peer::Adtb {"adtb"} else {"sqlite"},
            "workload":if config.lifecycle {"lifecycle"} else if config.requests == 8 {"counter-smoke"} else {"counter"},
            "n":config.n,"writers":config.writers,"generator_version":"baseline-v1",
            "offered_model":"finite-closed-loop-per-actor","build":build,"settings":self.settings,
            "window":window.map(|(start_ns,end_ns)|json!({"start_ns":start_ns,"end_ns":end_ns})),
            "plan":plan,"record_counts":{"calls":calls.len(),"attempts":attempts.len(),"workflows":workflows.len()},
            "phases":phases,"storage":{"active":self.active,"closed":self.closed},
            "validation":{"loaded":self.validation[0],"before_close":self.validation[1],
                "after_reopen":self.validation[2],"closed_form":self.validation[3]},"process_io":process_io
        });
        let temporary = root.join(".application.json.tmp");
        let mut file = File::create_new(&temporary).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut file, &value).map_err(|e| e.to_string())?;
        file.write_all(b"\n").map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(temporary, root.join("application.json")).map_err(|e| e.to_string())
    }
}

fn write_rows(root: &Path, name: &str, rows: impl Iterator<Item = Value>) -> Result<(), String> {
    let mut file = File::create_new(root.join(name)).map_err(|e| e.to_string())?;
    for row in rows {
        serde_json::to_writer(&mut file, &row).map_err(|e| e.to_string())?;
        file.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    file.sync_all().map_err(|e| e.to_string())
}

fn call_json(call: &CallSample) -> Value {
    json!({"actor":call.actor.0,"call":call.call.0,"action":action_name(call.action),
        "target":target(call.action).0,"workflow":workflow(call.action),
        "offered_ns":call.offered_ns,"ended_ns":call.ended_ns,
        "outcome":call.terminal.outcome(),"failure_kind":call.terminal.failure(),"attempts":call.attempts})
}

fn attempt_json(attempt: &AttemptSample) -> Value {
    json!({"actor":attempt.actor.0,"call":attempt.call.0,"attempt":attempt.attempt,
        "started_ns":attempt.started_ns,"ended_ns":attempt.ended_ns,"outcome":attempt.outcome})
}

fn span_json(span: &Span) -> Value {
    json!({"name":span.name,"actor":span.actor.map(|a| a.0),"start_ns":span.start_ns,"end_ns":span.end_ns})
}

fn workflow_rows(calls: &[&CallSample]) -> Vec<Value> {
    let mut groups: BTreeMap<_, Vec<&CallSample>> = BTreeMap::new();
    for call in calls {
        if let Some(id) = workflow(call.action) {
            groups.entry((call.actor.0, id)).or_default().push(call);
        }
    }
    groups.into_iter().map(|((actor, id), calls)| {
        let expected = [Action::Start(Id(id)),Action::ReadLive(Id(id)),Action::Finish(Id(id)),Action::ReadPlayed(Id(id))];
        let complete = calls.len() == expected.len() && calls.iter().zip(expected).all(|(c, a)| c.action == a && c.terminal == Terminal::Completed);
        let terminal = if complete { Terminal::Completed } else if calls.iter().any(|c| c.terminal == Terminal::Uncertain) { Terminal::Uncertain } else { Terminal::Failed("incomplete_workflow") };
        json!({"actor":actor,"workflow":id,"offered_ns":calls.first().expect("group contains its first call").offered_ns,
            "ended_ns":calls.last().expect("group contains its last call").ended_ns,"outcome":terminal.outcome(),"calls":calls.len()})
    }).collect()
}

#[cfg(test)]
#[path = "artifact_tests.rs"]
mod tests;
