use super::model::{self, Action, Failure, Output, Snapshot};
use super::{calls, schema};
use adtb::Key;
use adtb::typed::{Date, Instant};
use rusqlite::{Connection, ErrorCode, OpenFlags, Statement, TransactionBehavior, params};
use std::path::Path;

type Result<T> = std::result::Result<T, Failure>;

pub struct Sqlite {
    connection: Connection,
}

impl Sqlite {
    pub fn open(path: &Path, writers: usize, create: bool) -> Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE;
        let connection = Connection::open_with_flags(
            path,
            if create {
                flags | OpenFlags::SQLITE_OPEN_CREATE
            } else {
                flags
            },
        )?;
        connection.busy_timeout(std::time::Duration::from_millis(5000))?;
        if create {
            connection.pragma_update(None, "page_size", 8192)?;
        }
        connection.execute_batch(&format!("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA ignore_check_constraints=OFF; PRAGMA read_uncommitted=OFF; PRAGMA mmap_size=0; PRAGMA wal_autocheckpoint=1000; PRAGMA cache_size=-{};", 65536 / writers))?;
        for (name, expected) in [
            ("page_size", 8192),
            ("busy_timeout", 5000),
            ("synchronous", 2),
            ("foreign_keys", 1),
            ("ignore_check_constraints", 0),
            ("read_uncommitted", 0),
            ("mmap_size", 0),
            ("wal_autocheckpoint", 1000),
            (
                "cache_size",
                -65536 / i64::try_from(writers).map_err(|e| Failure::Fatal(e.to_string()))?,
            ),
        ] {
            let got: i64 = connection.pragma_query_value(None, name, |r| r.get(0))?;
            if got != expected {
                return Err(Failure::Fatal(format!(
                    "PRAGMA {name}: {got} != {expected}"
                )));
            }
        }
        let journal: String = connection.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        let identity: (String, String) =
            connection.query_row("SELECT sqlite_version(), sqlite_source_id()", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        if journal != "wal" || identity != ("3.54.1".into(), "2026-10-16 14:45:20 555e07ef33f69a0957d24d8e555b702464a131d6f1607f4c3c1e4551e8e6ca76".into()) {
            return Err(Failure::Fatal(format!("SQLite identity: {journal} {identity:?}")));
        }
        if create {
            connection.execute_batch(include_str!("../../fixtures/schema.sql"))?;
        }
        Ok(Self { connection })
    }

    pub fn settings(&self) -> Result<String> {
        let mut stmt = self.connection.prepare("PRAGMA compile_options")?;
        let options: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(format!(
            "SQLite=3.54.1 WAL FULL private-cache busy_timeout_ms=5000 native_busy_retries=unknown; compile_options={options:?}"
        ))
    }

    pub fn seed(&mut self, fixture: &Snapshot) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for c in &fixture.counters {
            tx.prepare("INSERT INTO counter VALUES(?,?)")?
                .execute(params![c.id, c.value])?;
        }
        for c in &fixture.campaigns {
            tx.prepare("INSERT INTO campaign VALUES(?,?)")?
                .execute(params![c.id, c.title])?;
        }
        for e in &fixture.entities {
            let (id, c, name, tag) = match e {
                schema::Entity::Party { id, campaign, name } => (id, campaign, name, "Party"),
                schema::Entity::Npc { id, campaign, name } => (id, campaign, name, "Npc"),
            };
            tx.prepare("INSERT INTO entity VALUES(?,?,?,?)")?
                .execute(params![id, c.0, name, tag])?;
        }
        for s in &fixture.sessions {
            let schema::Session::Planned {
                id,
                campaign,
                party,
                number,
                title,
                prep,
                planned_for,
            } = s
            else {
                return Err(Failure::Fatal("fixture must be Planned".into()));
            };
            tx.prepare("INSERT INTO session(id,campaign,party_id,party_tag,number,title,prep,tag,planned_for) VALUES(?,?,?,'Party',?,?,?,'Planned',?)")?.execute(params![id,campaign.0,party.0,number,title,prep,planned_for.map(|d| d.0)])?;
        }
        tx.commit()
            .map_err(|e| Failure::Ambiguous(format!("seed commit: {e:?}")))
    }

    pub fn prepare(&self) -> Result<Prepared<'_>> {
        Prepared::new(&self.connection)
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let mut stmt = self
            .connection
            .prepare("SELECT id,value FROM counter ORDER BY id")?;
        let counters = stmt
            .query_map([], |r| {
                Ok(schema::Counter {
                    id: r.get(0)?,
                    value: r.get(1)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        let mut stmt = self
            .connection
            .prepare("SELECT id,title FROM campaign ORDER BY id")?;
        let campaigns = stmt
            .query_map([], |r| {
                Ok(schema::Campaign {
                    id: r.get(0)?,
                    title: r.get(1)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        let mut stmt = self
            .connection
            .prepare("SELECT id,campaign,name,tag FROM entity ORDER BY id")?;
        let entities = stmt
            .query_map([], |r| {
                let (id, campaign, name) = (r.get(0)?, Key(r.get(1)?), r.get(2)?);
                match r.get::<_, String>(3)?.as_str() {
                    "Party" => Ok(schema::Entity::Party { id, campaign, name }),
                    "Npc" => Ok(schema::Entity::Npc { id, campaign, name }),
                    _ => Err(rusqlite::Error::InvalidQuery),
                }
            })?
            .collect::<std::result::Result<_, _>>()?;
        let mut stmt = self.connection.prepare("SELECT id,campaign,party_id,number,title,prep,tag,planned_for,tick,day,log,recap FROM session ORDER BY id")?;
        let sessions = stmt
            .query_map([], session)?
            .collect::<std::result::Result<_, _>>()?;
        Ok(Snapshot {
            counters,
            campaigns,
            entities,
            sessions,
        })
    }

    pub fn close(self) -> Result<()> {
        self.connection
            .close()
            .map_err(|(_, e)| Failure::Fatal(format!("close: {e:?}")))
    }
}

fn begin_error(error: rusqlite::Error) -> Failure {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
            Failure::Retry(format!("BEGIN: {error:?}"))
        }
        _ => Failure::from(error),
    }
}

fn session(r: &rusqlite::Row<'_>) -> rusqlite::Result<schema::Session> {
    let (id, campaign, party, number, title, prep) = (
        r.get(0)?,
        Key(r.get(1)?),
        Key(r.get(2)?),
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
    );
    match r.get::<_, String>(6)?.as_str() {
        "Planned" => Ok(schema::Session::Planned {
            id,
            campaign,
            party,
            number,
            title,
            prep,
            planned_for: r.get::<_, Option<i32>>(7)?.map(Date),
        }),
        "Live" => Ok(schema::Session::Live {
            id,
            campaign,
            party,
            number,
            title,
            prep,
            started: Instant(r.get(8)?),
        }),
        "Played" => Ok(schema::Session::Played {
            id,
            campaign,
            party,
            number,
            title,
            prep,
            played_on: Date(r.get(9)?),
            log: r.get(10)?,
            recap: r.get(11)?,
        }),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

pub struct Prepared<'a> {
    connection: &'a Connection,
    counter_read: Statement<'a>,
    counter_update: Statement<'a>,
    session_tag: Statement<'a>,
    start: Statement<'a>,
    finish: Statement<'a>,
    live: Statement<'a>,
    played: Statement<'a>,
}

impl<'a> Prepared<'a> {
    fn new(connection: &'a Connection) -> Result<Self> {
        Ok(Self { connection,
            counter_read: connection.prepare("SELECT value FROM counter WHERE id=?")?,
            counter_update: connection.prepare("UPDATE counter SET value=? WHERE id=?")?,
            session_tag: connection.prepare("SELECT tag FROM session WHERE id=?")?,
            start: connection.prepare("UPDATE session SET tag='Live',planned_for=NULL,tick=? WHERE id=?")?,
            finish: connection.prepare("UPDATE session SET tag='Played',tick=NULL,day=?,log=?,recap=? WHERE id=?")?,
            live: connection.prepare("SELECT s.id,s.number,s.title,e.name,s.tick FROM session s JOIN entity e ON e.id=s.party_id WHERE s.campaign=? AND s.tag='Live'")?,
            played: connection.prepare("SELECT s.id,s.number,s.title,e.name,s.day,s.log,s.recap,s.tag FROM session s JOIN entity e ON e.id=s.party_id WHERE s.id=?")?,
        })
    }

    pub fn call(&mut self, action: Action, remaining: std::time::Duration) -> Result<Output> {
        self.connection
            .busy_timeout(remaining.min(std::time::Duration::from_millis(5000)))?;
        let begin = match action {
            Action::ReadLive(_) | Action::ReadPlayed(_) => "BEGIN",
            _ => "BEGIN IMMEDIATE",
        };
        self.connection.execute_batch(begin).map_err(begin_error)?;
        match self.execute(action) {
            Ok(output) => {
                self.connection
                    .execute_batch("COMMIT")
                    .map_err(|e| Failure::Ambiguous(format!("COMMIT {action:?}: {e:?}")))?;
                Ok(output)
            }
            Err(error) => {
                self.connection
                    .execute_batch("ROLLBACK")
                    .map_err(|e| Failure::Ambiguous(format!("ROLLBACK {action:?}: {e:?}")))?;
                Err(error)
            }
        }
    }

    fn execute(&mut self, action: Action) -> Result<Output> {
        match action {
            Action::Bump(id) => {
                let old: i64 = self.counter_read.query_row([id.0], |r| r.get(0))?;
                let changed = self.counter_update.execute(params![old + 1, id.0])?;
                if changed != 1 {
                    return Err(Failure::Fatal(format!("bump changed {changed} rows")));
                }
                Ok(Output::Number(old + 1))
            }
            Action::Start(id) | Action::Finish(id) => {
                let tag: String = self.session_tag.query_row([id.0], |r| r.get(0))?;
                let (required, reason) = if matches!(action, Action::Start(_)) {
                    ("Planned", "Session is not planned")
                } else {
                    ("Live", "Session is not live")
                };
                if tag != required {
                    return Err(Failure::Refused(reason.into()));
                }
                let changed = if matches!(action, Action::Start(_)) {
                    self.start.execute(params![model::tick(id).0, id.0])?
                } else {
                    self.finish.execute(params![
                        model::day(id).0,
                        model::log(id),
                        model::recap(id),
                        id.0
                    ])?
                };
                if changed != 1 {
                    return Err(Failure::Fatal(format!("transition changed {changed} rows")));
                }
                Ok(Output::Unit)
            }
            Action::ReadLive(id) => {
                let stmt = &mut self.live;
                let rows = stmt
                    .query_map([model::campaign(id)], |r| {
                        Ok(calls::LiveRow {
                            id: Key(r.get(0)?),
                            number: r.get(1)?,
                            title: r.get(2)?,
                            party: r.get(3)?,
                            tick: Instant(r.get(4)?),
                        })
                    })?
                    .collect::<std::result::Result<_, _>>()?;
                Ok(Output::Live(rows))
            }
            Action::ReadPlayed(id) => {
                let stmt = &mut self.played;
                let rows = stmt
                    .query_map([id.0], |r| {
                        if r.get::<_, String>(7)? != "Played" {
                            return Ok(None);
                        }
                        Ok(Some(calls::PlayedRow {
                            id: Key(r.get(0)?),
                            number: r.get(1)?,
                            title: r.get(2)?,
                            party: r.get(3)?,
                            day: Date(r.get(4)?),
                            log: r.get(5)?,
                            recap: r.get(6)?,
                        }))
                    })?
                    .collect::<std::result::Result<_, _>>()?;
                Ok(Output::Played(rows))
            }
        }
    }
}
