use std::cell::Cell;

use super::memo::Memo;

/// Returns the value `memo` gives for `key`, counting each call of its maker in `made`.
fn get(memo: &Memo<u32, u32>, made: &Cell<u32>, key: u32) -> u32 {
    let v = memo.get_or_try_insert(key, || {
        made.set(made.get() + 1);
        Ok::<_, ()>(key * 10)
    });
    *v.expect("the maker succeeds")
}

#[test]
fn a_held_key_is_not_made_again() {
    let (memo, made) = (Memo::new(2), Cell::new(0));
    assert_eq!(10, get(&memo, &made, 1));
    assert_eq!(10, get(&memo, &made, 1));
    assert_eq!(1, made.get());
}

#[test]
fn the_least_recently_used_key_is_evicted_past_the_bound() {
    let (memo, made) = (Memo::new(2), Cell::new(0));
    get(&memo, &made, 1);
    get(&memo, &made, 2);
    get(&memo, &made, 1);
    get(&memo, &made, 3);
    assert_eq!(3, made.get());
    get(&memo, &made, 1);
    assert_eq!(3, made.get(), "1 was used after 2, so 3 evicted 2");
    get(&memo, &made, 2);
    assert_eq!(4, made.get(), "2 was evicted");
}

#[test]
fn a_failed_make_holds_nothing() {
    let (memo, made) = (Memo::new(2), Cell::new(0));
    assert_eq!(
        Err("no"),
        memo.get_or_try_insert(1, || Err("no")).map(|v| *v)
    );
    assert_eq!(0, memo.len());
    assert_eq!(10, get(&memo, &made, 1));
    assert_eq!(1, made.get());
}

#[test]
fn the_memo_never_holds_more_than_its_bound() {
    let (memo, made) = (Memo::new(2), Cell::new(0));
    for key in 0..10 {
        get(&memo, &made, key);
        assert!(memo.len() <= 2, "{} held after key {key}", memo.len());
    }
    assert_eq!(10, made.get());
    assert_eq!(2, memo.len());
}
