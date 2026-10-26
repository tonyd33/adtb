use std::thread;

use super::{Backend, LIVE, OsRuntime};
use crate::runtime::{ThreadId, Threads};

fn runtime() -> OsRuntime {
    OsRuntime::new(std::env::temp_dir(), Backend::Pool { threads: 1 })
}

fn is_live(id: ThreadId) -> bool {
    LIVE.read().unwrap().contains_key(&id)
}

#[expect(
    clippy::disallowed_methods,
    reason = "a host thread the runtime did not spawn"
)]
fn on_host_thread<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
    thread::spawn(body).join().unwrap()
}

#[test]
fn current_is_stable_within_a_thread_and_distinct_across_threads() {
    let rt = runtime();
    let me = rt.current();
    assert_eq!(me, rt.current());
    let other = on_host_thread(move || rt.current());
    assert_ne!(me, other);
}

#[test]
fn a_host_thread_is_forgotten_when_it_exits() {
    let rt = runtime();
    let rt2 = rt.clone();
    let id = on_host_thread(move || {
        let id = rt2.current();
        assert!(is_live(id));
        id
    });
    assert!(!is_live(id));
    rt.unpark(id);
}

#[test]
fn a_spawned_thread_is_forgotten_once_joined() {
    let rt = runtime();
    let rt2 = rt.clone();
    let id = rt.spawn("child", move || {
        let me = rt2.current();
        assert!(is_live(me));
    });
    rt.join(id);
    assert!(!is_live(id));
    assert!(!rt.threads.lock().contains_key(&id));
}

#[test]
fn a_spawned_thread_is_unparked_by_its_id_before_it_parks() {
    let rt = runtime();
    let rt2 = rt.clone();
    let id = rt.spawn("child", move || rt2.park());
    rt.unpark(id);
    rt.join(id);
}

#[test]
#[should_panic(expected = "was never handed out")]
fn unpark_of_an_id_never_handed_out_panics() {
    runtime().unpark(ThreadId(u64::MAX));
}
