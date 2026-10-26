mod common;

use adtb_isolation::{Anomaly, Dep, Edge, Expected, Malformed, Op, Outcome, Txn, TxnId, check};
use common::kinds;

fn append(key: i64, elem: i64) -> Op {
    Op::Append { key, elem }
}

fn read(key: i64, list: &[i64]) -> Op {
    Op::Read {
        key,
        list: list.to_vec(),
    }
}

fn ok(id: u64, ops: Vec<Op>) -> Txn {
    Txn::new(id, Outcome::Committed, ops)
}

fn failed(id: u64, ops: Vec<Op>) -> Txn {
    Txn::new(id, Outcome::Aborted, ops)
}

fn in_doubt(id: u64, ops: Vec<Op>) -> Txn {
    Txn::new(id, Outcome::InDoubt, ops)
}

const X: i64 = 1;
const Y: i64 = 2;

fn edge(from: u64, to: u64, key: i64, dep: Dep) -> Edge {
    Edge {
        from: TxnId(from),
        to: TxnId(to),
        key,
        dep,
    }
}

#[test]
fn empty_history_passes() {
    assert_eq!(check(&[]), Ok(vec![]));
}

#[test]
fn serial_appends_pass() {
    let h = [
        ok(1, vec![append(X, 1)]),
        ok(2, vec![read(X, &[1]), append(X, 2)]),
        ok(3, vec![read(X, &[1, 2]), read(Y, &[])]),
    ];
    assert_eq!(check(&h), Ok(vec![]));
}

#[test]
fn concurrent_but_serializable_passes() {
    let h = [
        ok(1, vec![read(X, &[]), append(Y, 1)]),
        ok(2, vec![read(Y, &[]), read(X, &[])]),
        ok(3, vec![append(X, 1), read(Y, &[1])]),
        ok(4, vec![read(X, &[1]), read(Y, &[1])]),
    ];
    assert_eq!(check(&h), Ok(vec![]));
}

#[test]
fn own_appends_are_visible_to_own_reads() {
    let h = [
        ok(
            1,
            vec![append(X, 1), read(X, &[1]), append(X, 2), read(X, &[1, 2])],
        ),
        ok(2, vec![read(X, &[1, 2]), append(X, 3), read(X, &[1, 2, 3])]),
    ];
    assert_eq!(check(&h), Ok(vec![]));
}

#[test]
fn adya_wcycle_is_g0() {
    let h = [
        ok(1, vec![append(X, 1), append(Y, 1)]),
        ok(2, vec![append(X, 2), append(Y, 2)]),
        ok(3, vec![read(X, &[1, 2]), read(Y, &[2, 1])]),
    ];
    let cycle = vec![
        edge(1, 2, X, Dep::Ww { prior: 1, next: 2 }),
        edge(2, 1, Y, Dep::Ww { prior: 2, next: 1 }),
    ];
    assert_eq!(check(&h), Ok(vec![Anomaly::G0 { cycle }]));
}

#[test]
fn interleaved_appends_of_one_transaction_are_g0() {
    let h = [
        ok(1, vec![append(X, 1), append(X, 3)]),
        ok(2, vec![append(X, 2)]),
        ok(3, vec![read(X, &[1, 2, 3])]),
    ];
    assert_eq!(kinds(&h), ["G0"]);
}

#[test]
fn reading_an_aborted_append_is_g1a() {
    let h = [failed(1, vec![append(X, 1)]), ok(2, vec![read(X, &[1])])];
    let g1a = Anomaly::G1a {
        reader: TxnId(2),
        writer: TxnId(1),
        key: X,
        elem: 1,
    };
    assert_eq!(check(&h), Ok(vec![g1a]));
}

#[test]
fn aborted_reader_of_an_aborted_append_passes() {
    let h = [
        failed(1, vec![append(X, 1)]),
        failed(2, vec![read(X, &[1])]),
    ];
    assert_eq!(check(&h), Ok(vec![]));
}

#[test]
fn reading_an_intermediate_append_is_g1b() {
    let h = [
        ok(1, vec![append(X, 1), append(X, 2)]),
        ok(2, vec![read(X, &[1])]),
        ok(3, vec![read(X, &[1, 2])]),
    ];
    let g1b = Anomaly::G1b {
        reader: TxnId(2),
        writer: TxnId(1),
        key: X,
        elem: 1,
    };
    assert_eq!(check(&h), Ok(vec![g1b]));
}

#[test]
fn reading_each_others_appends_is_g1c() {
    let h = [
        ok(1, vec![append(X, 1), read(Y, &[1])]),
        ok(2, vec![append(Y, 1), read(X, &[1])]),
    ];
    let cycle = vec![
        edge(1, 2, X, Dep::Wr { elem: 1 }),
        edge(2, 1, Y, Dep::Wr { elem: 1 }),
    ];
    assert_eq!(check(&h), Ok(vec![Anomaly::G1c { cycle }]));
}

#[test]
fn read_skew_is_g_single() {
    let h = [
        ok(0, vec![append(X, 1), append(Y, 1)]),
        ok(1, vec![read(X, &[1]), read(Y, &[1, 2])]),
        ok(2, vec![append(X, 2), append(Y, 2)]),
        ok(3, vec![read(X, &[1, 2]), read(Y, &[1, 2])]),
    ];
    let cycle = vec![
        edge(
            1,
            2,
            X,
            Dep::Rw {
                read: Some(1),
                missed: 2,
            },
        ),
        edge(2, 1, Y, Dep::Wr { elem: 2 }),
    ];
    assert_eq!(check(&h), Ok(vec![Anomaly::GSingle { cycle }]));
}

#[test]
fn lost_update_is_g_single() {
    let h = [
        ok(1, vec![read(X, &[]), append(X, 1)]),
        ok(2, vec![read(X, &[]), append(X, 2)]),
        ok(3, vec![read(X, &[1, 2])]),
    ];
    let cycle = vec![
        edge(
            2,
            1,
            X,
            Dep::Rw {
                read: None,
                missed: 1,
            },
        ),
        edge(1, 2, X, Dep::Ww { prior: 1, next: 2 }),
    ];
    assert_eq!(check(&h), Ok(vec![Anomaly::GSingle { cycle }]));
}

#[test]
fn write_skew_is_g2() {
    let h = [
        ok(1, vec![read(X, &[]), read(Y, &[]), append(X, 1)]),
        ok(2, vec![read(X, &[]), read(Y, &[]), append(Y, 1)]),
        ok(3, vec![read(X, &[1]), read(Y, &[1])]),
    ];
    let cycle = vec![
        edge(
            1,
            2,
            Y,
            Dep::Rw {
                read: None,
                missed: 1,
            },
        ),
        edge(
            2,
            1,
            X,
            Dep::Rw {
                read: None,
                missed: 1,
            },
        ),
    ];
    assert_eq!(check(&h), Ok(vec![Anomaly::G2 { cycle }]));
}

#[test]
fn write_skew_with_one_side_aborted_passes() {
    let h = [
        ok(1, vec![read(X, &[]), read(Y, &[]), append(X, 1)]),
        failed(2, vec![read(X, &[]), read(Y, &[]), append(Y, 1)]),
        ok(3, vec![read(X, &[1]), read(Y, &[])]),
    ];
    assert_eq!(check(&h), Ok(vec![]));
}

#[test]
fn three_way_anti_dependency_cycle_is_g2() {
    let z = 3;
    let h = [
        ok(1, vec![read(X, &[]), append(Y, 1)]),
        ok(2, vec![read(Y, &[]), append(z, 1)]),
        ok(3, vec![read(z, &[]), append(X, 1)]),
        ok(4, vec![read(X, &[1]), read(Y, &[1]), read(z, &[1])]),
    ];
    assert_eq!(kinds(&h), ["G2"]);
}

#[test]
fn separate_components_each_report_a_cycle() {
    let (a, b) = (3, 4);
    let h = [
        ok(1, vec![read(X, &[]), read(Y, &[]), append(X, 1)]),
        ok(2, vec![read(X, &[]), read(Y, &[]), append(Y, 1)]),
        ok(3, vec![read(a, &[]), read(b, &[]), append(a, 1)]),
        ok(4, vec![read(a, &[]), read(b, &[]), append(b, 1)]),
        ok(
            5,
            vec![read(X, &[1]), read(Y, &[1]), read(a, &[1]), read(b, &[1])],
        ),
    ];
    assert_eq!(kinds(&h), ["G2", "G2"]);
}

#[test]
fn read_missing_an_own_append_is_internal() {
    let h = [ok(1, vec![append(X, 1), read(X, &[])])];
    let internal = Anomaly::Internal {
        txn: TxnId(1),
        key: X,
        expected: Expected::Suffix(vec![1]),
        read: vec![],
    };
    assert_eq!(check(&h), Ok(vec![internal]));
}

#[test]
fn reread_that_changed_is_internal_and_g_single() {
    let h = [
        ok(1, vec![append(X, 1)]),
        ok(2, vec![read(X, &[]), read(X, &[1])]),
    ];
    assert_eq!(kinds(&h), ["internal", "G-single"]);
}

#[test]
fn read_of_an_unwritten_element_is_garbage() {
    let h = [ok(1, vec![read(X, &[7])])];
    let garbage = Anomaly::Garbage {
        reader: TxnId(1),
        key: X,
        elem: 7,
    };
    assert_eq!(check(&h), Ok(vec![garbage]));
}

#[test]
fn read_holding_an_element_twice_is_duplicate() {
    let h = [ok(1, vec![append(X, 1)]), ok(2, vec![read(X, &[1, 1])])];
    assert_eq!(kinds(&h), ["duplicate"]);
}

#[test]
fn reads_that_diverge_are_incompatible() {
    let h = [
        ok(1, vec![append(X, 1)]),
        ok(2, vec![append(X, 2)]),
        ok(3, vec![append(X, 3)]),
        ok(4, vec![read(X, &[1, 2])]),
        ok(5, vec![read(X, &[1, 3])]),
    ];
    let incompatible = Anomaly::IncompatibleOrder {
        key: X,
        left: vec![1, 3],
        right: vec![1, 2],
    };
    assert_eq!(check(&h), Ok(vec![incompatible]));
}

#[test]
fn observed_in_doubt_transaction_counts_as_committed() {
    let h = [
        in_doubt(1, vec![read(X, &[]), append(Y, 1)]),
        ok(2, vec![read(Y, &[]), append(X, 1)]),
        ok(3, vec![read(X, &[1]), read(Y, &[1])]),
    ];
    assert_eq!(kinds(&h), ["G2"]);
}

#[test]
fn unobserved_in_doubt_transaction_is_ignored() {
    let h = [
        in_doubt(1, vec![read(X, &[]), append(Y, 1)]),
        ok(2, vec![read(Y, &[]), append(X, 1)]),
        ok(3, vec![read(X, &[1]), read(Y, &[])]),
    ];
    assert_eq!(check(&h), Ok(vec![]));
}

#[test]
fn in_doubt_transactions_observed_through_each_other_count() {
    let h = [
        in_doubt(1, vec![append(X, 1)]),
        in_doubt(2, vec![read(X, &[1]), append(Y, 1)]),
        ok(3, vec![read(Y, &[1]), read(X, &[])]),
    ];
    assert_eq!(kinds(&h), ["G-single"]);
}

#[test]
fn duplicate_transaction_id_is_malformed() {
    let h = [ok(1, vec![]), ok(1, vec![])];
    assert_eq!(check(&h), Err(Malformed::DuplicateTxn(TxnId(1))));
}

#[test]
fn duplicate_append_is_malformed() {
    let h = [ok(1, vec![append(X, 1)]), failed(2, vec![append(X, 1)])];
    let dup = Malformed::DuplicateAppend {
        key: X,
        elem: 1,
        first: TxnId(1),
        second: TxnId(2),
    };
    assert_eq!(check(&h), Err(dup));
}

#[test]
fn same_element_on_different_keys_is_well_formed() {
    let h = [
        ok(1, vec![append(X, 1), append(Y, 1)]),
        ok(2, vec![read(X, &[1]), read(Y, &[1])]),
    ];
    assert_eq!(check(&h), Ok(vec![]));
}
