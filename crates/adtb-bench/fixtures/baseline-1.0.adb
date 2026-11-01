bundle baseline-1.0 pins 1 (a0fcbcb5abe71ac88bce427a90a4b99d0cc887e09fce91611d904d118de97bb5) edition "2026-10"
import baseline
query counters () -> Set Counter = from c in counters select c
query campaigns () -> Set Campaign = from c in campaigns select c
query entities () -> Set Entity = from e in entities select e
query sessions () -> Set Session = from s in sessions select s
txn seed_counter (id: I64) -> () =
  transact
    insert counters { id: id, value: 0 }
txn bump (k: Key counters) -> I64 =
  transact
    if not exists c in counters where key c == k then fail "Counter missing" else skip
    let old = sum(c.value for c in counters where key c == k)
    update c in counters where key c == k set Counter { c with value: old + 1 }
    return old + 1
txn seed_campaign (id: I64, title: Text) -> () =
  transact
    insert campaigns { id: id, title: title }
txn seed_party (id: I64, c: Key campaigns, name: Text) -> () =
  transact
    insert entities.Party { id: id, campaign: c, name: name }
txn seed_npc (id: I64, c: Key campaigns, name: Text) -> () =
  transact
    insert entities.Npc { id: id, campaign: c, name: name }
txn seed_session (id: I64, c: Key campaigns, p: Key entities.Party, number: I64, title: Text, prep: Text) -> () =
  transact
    insert sessions.Planned { id: id, campaign: c, party: p, number: number, title: title, prep: prep, planned_for: None }
txn start (s: Key sessions, tick: Instant) -> () =
  transact
    if not exists x in sessions.Planned where key x == s then fail "Session is not planned" else skip
    update x in sessions.Planned where key x == s set Live { started: tick }
txn finish (s: Key sessions, day: Date, log: Text, recap: Text) -> () =
  transact
    if not exists x in sessions.Live where key x == s then fail "Session is not live" else skip
    update x in sessions.Live where key x == s set Played { played_on: day, log: log, recap: recap }
query live (c: Key campaigns) -> Set ({ id: Key sessions, number: I64, title: Text, party: Text, tick: Instant }) =
  from s in sessions.Live where s.campaign == c
  select { id: key s, number: s.number, title: s.title, party: s.party.name, tick: s.started }
query played (k: Key sessions) -> Set (Option ({ id: Key sessions, number: I64, title: Text, party: Text, day: Date, log: Text, recap: Text })) =
  from s in sessions where key s == k
  select match s
    | Played p -> Some { value: { id: key p, number: p.number, title: p.title, party: p.party.name, day: p.played_on, log: p.log, recap: p.recap } }
    | Planned _ | Live _ -> None
