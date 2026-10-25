mod common;

use adtb_isolation::{
    ABORTED_READ, CIRCULAR_FLOW, INTERMEDIATE_READ, LOST_UPDATE, Outcome, READ_SKEW, SCENARIOS,
    Scenario, Step, TxnId, WRITE_SKEW, check, run,
};
use common::{Memory, Mode, kinds};

fn under(mode: Mode, scenario: Scenario) -> Vec<&'static str> {
    kinds(&run(&mut Memory::new(mode), scenario.steps))
}

#[test]
fn serializable_database_runs_every_scenario_without_anomaly() {
    for scenario in SCENARIOS {
        let history = run(&mut Memory::new(Mode::Optimistic), scenario.steps);
        assert_eq!(check(&history), Ok(vec![]), "{}", scenario.name);
    }
}

#[test]
fn serializable_database_aborts_one_side_of_write_skew() {
    let history = run(&mut Memory::new(Mode::Optimistic), WRITE_SKEW.steps);
    let outcome = |t| history.iter().find(|x| x.id == TxnId(t)).unwrap().outcome;
    let sides = [outcome(1), outcome(2)];
    assert!(sides.contains(&Outcome::Aborted), "{sides:?}");
    assert!(sides.contains(&Outcome::Committed), "{sides:?}");
}

#[test]
fn snapshot_isolation_allows_write_skew() {
    assert_eq!(under(Mode::Snapshot, WRITE_SKEW), ["G2"]);
}

#[test]
fn snapshot_isolation_prevents_lost_update() {
    assert_eq!(under(Mode::Snapshot, LOST_UPDATE), Vec::<&str>::new());
}

#[test]
fn read_committed_loses_updates() {
    assert_eq!(under(Mode::ReadCommitted, LOST_UPDATE), ["G-single"]);
}

#[test]
fn snapshot_isolation_prevents_read_skew() {
    assert_eq!(under(Mode::Snapshot, READ_SKEW), Vec::<&str>::new());
}

#[test]
fn read_committed_allows_read_skew() {
    assert_eq!(under(Mode::ReadCommitted, READ_SKEW), ["G-single"]);
}

#[test]
fn read_committed_prevents_g0_and_g1() {
    for scenario in SCENARIOS {
        let found = under(Mode::ReadCommitted, *scenario);
        assert!(
            found.iter().all(|k| *k == "G-single" || *k == "G2"),
            "{}: {found:?}",
            scenario.name
        );
    }
}

#[test]
fn read_committed_allows_write_skew_through_unseen_appends() {
    assert_eq!(under(Mode::ReadCommitted, CIRCULAR_FLOW), ["G2"]);
}

#[test]
fn read_uncommitted_allows_aborted_reads() {
    assert_eq!(under(Mode::Uncommitted, ABORTED_READ), ["G1a"]);
}

#[test]
fn read_uncommitted_allows_intermediate_reads() {
    assert_eq!(under(Mode::Uncommitted, INTERMEDIATE_READ), ["G1b"]);
}

#[test]
fn read_uncommitted_allows_circular_information_flow() {
    assert_eq!(under(Mode::Uncommitted, CIRCULAR_FLOW), ["G1c"]);
}

#[test]
fn transaction_left_open_is_aborted() {
    let history = run(
        &mut Memory::new(Mode::Snapshot),
        &[Step::Begin(1), Step::Append(1, 1, 1)],
    );
    assert_eq!(history[0].outcome, Outcome::Aborted);
}
