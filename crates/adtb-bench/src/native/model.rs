use super::{calls, schema};
use adtb::Key;
use adtb::typed::{Date, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Id(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Bump(Id),
    Start(Id),
    ReadLive(Id),
    Finish(Id),
    ReadPlayed(Id),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    Number(i64),
    Unit,
    Live(Vec<calls::LiveRow>),
    Played(Vec<Option<calls::PlayedRow>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Retry(String),
    Refused(String),
    Ambiguous(String),
    Fatal(String),
}

impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Self {
        Self::Fatal(format!("{error:?}"))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub counters: Vec<schema::Counter>,
    pub campaigns: Vec<schema::Campaign>,
    pub entities: Vec<schema::Entity>,
    pub sessions: Vec<schema::Session>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prefix {
    Planned,
    Started,
    ReadLive,
    Finished,
    ReadPlayed,
}

impl Prefix {
    pub fn advance(self, action: Action) -> Result<Self, String> {
        match (self, action) {
            (Self::Planned, Action::Start(_)) => Ok(Self::Started),
            (Self::Started, Action::ReadLive(_)) => Ok(Self::ReadLive),
            (Self::ReadLive, Action::Finish(_)) => Ok(Self::Finished),
            (Self::Finished, Action::ReadPlayed(_)) => Ok(Self::ReadPlayed),
            _ => Err(format!("out-of-order acknowledgement: {self:?} {action:?}")),
        }
    }
}

pub fn campaign(id: Id) -> i64 {
    1 + (id.0 - 1) % 4
}
pub fn tick(id: Id) -> Instant {
    Instant(1_800_000_000_000_000 + id.0)
}
pub fn day(id: Id) -> Date {
    Date(20_000 + i32::try_from(id.0 % 365).expect("days fit i32"))
}
pub fn log(id: Id) -> String {
    format!("log-{}", id.0)
}
pub fn recap(id: Id) -> String {
    format!("recap-{}", id.0)
}
pub fn permutation(n: i64) -> Vec<Id> {
    (0..n).map(|i| Id(1 + (i * 37) % n)).collect()
}

pub fn session(id: Id, prefix: Prefix) -> schema::Session {
    let campaign = Key(campaign(id));
    let party = Key(campaign.0);
    let number = id.0;
    let title = format!("session-{}", id.0);
    let prep = format!("prep-{}", id.0);
    match prefix {
        Prefix::Planned => schema::Session::Planned {
            id: id.0,
            campaign,
            party,
            number,
            title,
            prep,
            planned_for: None,
        },
        Prefix::Started | Prefix::ReadLive => schema::Session::Live {
            id: id.0,
            campaign,
            party,
            number,
            title,
            prep,
            started: tick(id),
        },
        Prefix::Finished | Prefix::ReadPlayed => schema::Session::Played {
            id: id.0,
            campaign,
            party,
            number,
            title,
            prep,
            played_on: day(id),
            log: log(id),
            recap: recap(id),
        },
    }
}

pub fn expected_output(action: Action) -> Output {
    let id = match action {
        Action::Start(_) | Action::Finish(_) => return Output::Unit,
        Action::ReadLive(id) | Action::ReadPlayed(id) => id,
        Action::Bump(_) => unreachable!("counter output depends on acknowledged increments"),
    };
    let party = format!("party-{}", campaign(id));
    let title = format!("session-{}", id.0);
    match action {
        Action::ReadLive(_) => Output::Live(vec![calls::LiveRow {
            id: Key(id.0),
            number: id.0,
            title,
            party,
            tick: tick(id),
        }]),
        Action::ReadPlayed(_) => Output::Played(vec![Some(calls::PlayedRow {
            id: Key(id.0),
            number: id.0,
            title,
            party,
            day: day(id),
            log: log(id),
            recap: recap(id),
        })]),
        _ => unreachable!("only lifecycle reads reach projection construction"),
    }
}

pub fn fixture(n: i64, lifecycle: bool) -> Snapshot {
    let counters = if lifecycle {
        Vec::new()
    } else {
        (1..=n).map(|id| schema::Counter { id, value: 0 }).collect()
    };
    let campaigns = if lifecycle {
        (1..=4)
            .map(|id| schema::Campaign {
                id,
                title: format!("campaign-{id}"),
            })
            .collect()
    } else {
        Vec::new()
    };
    let entities = if lifecycle {
        (1..=4)
            .flat_map(|id| {
                [
                    schema::Entity::Party {
                        id,
                        campaign: Key(id),
                        name: format!("party-{id}"),
                    },
                    schema::Entity::Npc {
                        id: id + 4,
                        campaign: Key(id),
                        name: format!("npc-{id}"),
                    },
                ]
            })
            .collect()
    } else {
        Vec::new()
    };
    let sessions = if lifecycle {
        (1..=n).map(|id| session(Id(id), Prefix::Planned)).collect()
    } else {
        Vec::new()
    };
    let mut state = Snapshot {
        counters,
        campaigns,
        entities,
        sessions,
    };
    state.sort();
    state
}

impl Snapshot {
    pub fn sort(&mut self) {
        self.counters.sort_by_key(|c| c.id);
        self.campaigns.sort_by_key(|c| c.id);
        self.entities.sort_by_key(|e| match e {
            schema::Entity::Party { id, .. } | schema::Entity::Npc { id, .. } => *id,
        });
        self.sessions.sort_by_key(session_id);
    }
}

pub fn session_id(s: &schema::Session) -> i64 {
    match s {
        schema::Session::Planned { id, .. }
        | schema::Session::Live { id, .. }
        | schema::Session::Played { id, .. } => *id,
    }
}
