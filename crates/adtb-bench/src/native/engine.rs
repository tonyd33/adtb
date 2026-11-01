use super::model::{self, Action, Failure, Output, Snapshot};
use super::{calls, schema};
use adtb::typed::{Bundle, HostValue};
use adtb::{
    BundleDef, CallOptions, CreateOptions, Database, Durability, Key, Log, OpenOptions, RetryReason,
};
use adtb_sys::os::{Backend, OsRuntime};
use std::path::Path;

type Result<T> = std::result::Result<T, Failure>;

pub struct Adtb {
    pub db: Database,
    pub clock: OsRuntime,
    query_budget: u64,
    bundle: Bundle<calls::Bundle>,
}

impl Adtb {
    pub fn open(root: &Path, writers: usize, create: bool) -> Result<Self> {
        let rt = OsRuntime::try_new(root, Backend::Pool { threads: 4 })
            .map_err(|e| Failure::Fatal(e.to_string()))?;
        let options = OpenOptions {
            create: create.then(CreateOptions::default),
            pool_bytes: Some(64 * 1024 * 1024),
            query_memory_bytes: Some(
                u64::try_from(writers).map_err(|e| Failure::Fatal(e.to_string()))?
                    * 64
                    * 1024
                    * 1024,
            ),
            workers: Some(4),
            registrations: vec![
                calls::BUNDLE
                    .registration()
                    .map_err(|e| Failure::Fatal(format!("register: {e:?}")))?,
            ],
            ..OpenOptions::default()
        };
        let db = Database::open_in(
            rt.clone(),
            "baseline.adt",
            &Log::new(schema::LOG.iter().copied()),
            &options,
        )
        .map_err(|e| Failure::Fatal(format!("open: {e:?}")))?;
        let bundle = db
            .bundle(&calls::BUNDLE)
            .map_err(|e| Failure::Fatal(format!("bind: {e:?}")))?;
        let out = Self {
            db,
            bundle,
            clock: rt,
            query_budget: options
                .query_memory_bytes
                .ok_or_else(|| Failure::Fatal("query budget must be explicit".into()))?,
        };
        if create {
            out.db
                .query_text(
                    "set setting ScrubInterval = { every: None }",
                    &[],
                    &options_call(),
                )
                .map_err(from_error)?;
        }
        Ok(out)
    }

    pub fn actor(&self) -> Result<Self> {
        let db = self.db.clone();
        let bundle = db
            .bundle(&calls::BUNDLE)
            .map_err(|e| Failure::Fatal(format!("bind actor: {e:?}")))?;
        Ok(Self {
            db,
            bundle,
            clock: self.clock.clone(),
            query_budget: self.query_budget,
        })
    }

    #[cfg(test)]
    pub fn control(
        &self,
        text: &str,
        params: &[(&str, adtb_lang::types::Ty, adtb::Value)],
    ) -> std::result::Result<Vec<adtb::Value>, adtb::Error> {
        self.db
            .query_text(text, params, &options_call())?
            .into_iter()
            .flatten()
            .collect()
    }

    pub fn settings(&self) -> Result<String> {
        for (name, expected) in [
            ("pool_bytes", 64 * 1024 * 1024),
            ("query_memory_bytes", self.query_budget),
            ("workers", 4),
        ] {
            let text =
                format!("from p in sys::process_options where p.name == \"{name}\" select p.value");
            let mut values = Vec::new();
            for set in self
                .db
                .query_text(&text, &[], &options_call())
                .map_err(from_error)?
            {
                for row in set {
                    values.push(
                        String::from_value(&row.map_err(from_error)?)
                            .ok_or_else(|| Failure::Fatal("process setting must be text".into()))?,
                    );
                }
            }
            if values != [expected.to_string()] {
                return Err(Failure::Fatal(format!(
                    "process setting {name}: {values:?} != {expected}"
                )));
            }
        }
        let mut settings = Vec::new();
        for text in [
            "from p in sys::process_options select p",
            "from s in catalog::settings select s",
        ] {
            let sets = self
                .db
                .query_text(text, &[], &options_call())
                .map_err(from_error)?;
            for set in sets {
                for row in set {
                    settings.push(format!("{:?}", row.map_err(from_error)?));
                }
            }
        }
        Ok(format!("adtb Pool4 Local retry=0; settings={settings:?}"))
    }

    pub fn seed(&self, fixture: &Snapshot) -> Result<()> {
        let options = options_call();
        for c in &fixture.counters {
            self.bundle
                .call(calls::seed_counter, (c.id,), &options)
                .map_err(convert)?;
        }
        for c in &fixture.campaigns {
            self.bundle
                .call(calls::seed_campaign, (c.id, c.title.clone()), &options)
                .map_err(convert)?;
        }
        for e in &fixture.entities {
            match e {
                schema::Entity::Party { id, campaign, name } => {
                    self.bundle
                        .call(
                            calls::seed_party,
                            (*id, campaign.clone(), name.clone()),
                            &options,
                        )
                        .map_err(convert)?;
                }
                schema::Entity::Npc { id, campaign, name } => {
                    self.bundle
                        .call(
                            calls::seed_npc,
                            (*id, campaign.clone(), name.clone()),
                            &options,
                        )
                        .map_err(convert)?;
                }
            }
        }
        for s in &fixture.sessions {
            let schema::Session::Planned {
                id,
                campaign,
                party,
                number,
                title,
                prep,
                ..
            } = s
            else {
                return Err(Failure::Fatal("fixture must be Planned".into()));
            };
            self.bundle
                .call(
                    calls::seed_session,
                    (
                        *id,
                        campaign.clone(),
                        party.clone(),
                        *number,
                        title.clone(),
                        prep.clone(),
                    ),
                    &options,
                )
                .map_err(convert)?;
        }
        Ok(())
    }

    pub fn call(
        &self,
        action: Action,
        remaining: std::time::Duration,
    ) -> Result<(Output, Option<adtb::Position>)> {
        let mut options = options_call();
        options.limit = Some(remaining.min(std::time::Duration::from_secs(10)));
        match action {
            Action::Bump(id) => {
                let result = self
                    .bundle
                    .call(calls::bump, (Key(id.0),), &options)
                    .map_err(convert)?;
                if result.attempts != 1 {
                    return Err(Failure::Fatal(format!(
                        "internal attempts {} != 1",
                        result.attempts
                    )));
                }
                Ok((Output::Number(result.value), Some(result.position)))
            }
            Action::Start(id) => {
                let result = self
                    .bundle
                    .call(calls::start, (Key(id.0), model::tick(id)), &options)
                    .map_err(convert)?;
                if result.attempts != 1 {
                    return Err(Failure::Fatal(format!(
                        "internal attempts {} != 1",
                        result.attempts
                    )));
                }
                Ok((Output::Unit, Some(result.position)))
            }
            Action::Finish(id) => {
                let result = self
                    .bundle
                    .call(
                        calls::finish,
                        (Key(id.0), model::day(id), model::log(id), model::recap(id)),
                        &options,
                    )
                    .map_err(convert)?;
                if result.attempts != 1 {
                    return Err(Failure::Fatal(format!(
                        "internal attempts {} != 1",
                        result.attempts
                    )));
                }
                Ok((Output::Unit, Some(result.position)))
            }
            Action::ReadLive(id) => {
                let rows = self
                    .bundle
                    .call(calls::live, (Key(model::campaign(id)),), &options)
                    .map_err(convert)?;
                Ok((
                    Output::Live(
                        rows.collect::<std::result::Result<_, _>>()
                            .map_err(convert)?,
                    ),
                    None,
                ))
            }
            Action::ReadPlayed(id) => {
                let rows = self
                    .bundle
                    .call(calls::played, (Key(id.0),), &options)
                    .map_err(convert)?;
                Ok((
                    Output::Played(
                        rows.collect::<std::result::Result<_, _>>()
                            .map_err(convert)?,
                    ),
                    None,
                ))
            }
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let options = options_call();
        let counters = self
            .bundle
            .call(calls::counters, (), &options)
            .map_err(convert)?
            .collect::<std::result::Result<_, _>>()
            .map_err(convert)?;
        let campaigns = self
            .bundle
            .call(calls::campaigns, (), &options)
            .map_err(convert)?
            .collect::<std::result::Result<_, _>>()
            .map_err(convert)?;
        let entities = self
            .bundle
            .call(calls::entities, (), &options)
            .map_err(convert)?
            .collect::<std::result::Result<_, _>>()
            .map_err(convert)?;
        let sessions = self
            .bundle
            .call(calls::sessions, (), &options)
            .map_err(convert)?
            .collect::<std::result::Result<_, _>>()
            .map_err(convert)?;
        let mut state = Snapshot {
            counters,
            campaigns,
            entities,
            sessions,
        };
        state.sort();
        Ok(state)
    }

    pub fn close(self) -> Result<()> {
        drop(self.bundle);
        self.db.close().map_err(from_error)
    }
}

pub fn options_call() -> CallOptions {
    CallOptions {
        durability: Durability::Local,
        retry: Some(0),
        limit: Some(std::time::Duration::from_secs(10)),
        ..CallOptions::default()
    }
}

fn from_error(error: adtb::Error) -> Failure {
    convert(error.into())
}

fn convert(error: calls::Error) -> Failure {
    match error {
        calls::Error::Retry {
            reason: RetryReason::Serialization { txn, on },
        } => Failure::Retry(format!("serialization {txn:?} {on}")),
        calls::Error::Rejected {
            reason: adtb::RejectReason::UserError { message },
        } => Failure::Refused(message),
        error @ calls::Error::Fatal { .. } => Failure::Ambiguous(format!("{error:?}")),
        other => Failure::Fatal(format!("{other:?}")),
    }
}
