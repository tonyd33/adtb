use std::thread;

use super::{Backend, Faults, LIVE, OsRuntime, Unavailable};
use crate::runtime::{ThreadId, Threads};

fn runtime() -> OsRuntime {
    OsRuntime::new(std::env::temp_dir(), Backend::Pool { threads: 1 })
}

fn is_live(id: ThreadId) -> bool {
    LIVE.read().unwrap().contains_key(&id)
}

#[expect(
    clippy::disallowed_methods,
    reason = "goes if a foreign thread can be observed without spawning one"
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

/// Make every later `io_uring_setup` on the calling thread fail with `EPERM`, as a sandbox does.
fn deny_io_uring_setup() {
    let nr = u32::try_from(libc::SYS_io_uring_setup).unwrap();
    let mut filter = [
        bpf(libc::BPF_LD | libc::BPF_W | libc::BPF_ABS, 0, 0, 0),
        bpf(libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K, 0, 1, nr),
        bpf(
            libc::BPF_RET | libc::BPF_K,
            0,
            0,
            libc::SECCOMP_RET_ERRNO | libc::EPERM.cast_unsigned(),
        ),
        bpf(libc::BPF_RET | libc::BPF_K, 0, 0, libc::SECCOMP_RET_ALLOW),
    ];
    let prog = libc::sock_fprog {
        len: u16::try_from(filter.len()).unwrap(),
        filter: filter.as_mut_ptr(),
    };
    // SAFETY: PR_SET_NO_NEW_PRIVS takes no pointers.
    let r = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    assert_eq!(0, r, "{}", std::io::Error::last_os_error());
    // SAFETY: `prog` and the filter it points to outlive the call, which copies them.
    let r = unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER,
            &raw const prog,
        )
    };
    assert_eq!(0, r, "{}", std::io::Error::last_os_error());
}

fn bpf(code: u32, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
    libc::sock_filter {
        code: u16::try_from(code).unwrap(),
        jt,
        jf,
        k,
    }
}

#[test]
fn a_denied_io_uring_is_refused_at_construction() {
    let refused = on_host_thread(|| {
        deny_io_uring_setup();
        OsRuntime::try_new(std::env::temp_dir(), Backend::Uring { entries: 8 }).map(drop)
    });
    match refused {
        Err(Unavailable::Refused(e)) => assert_eq!(Some(libc::EPERM), e.raw_os_error()),
        other => panic!("a denied io_uring_setup gave {other:?}"),
    }
}

#[test]
fn the_pool_is_the_fallback_where_io_uring_is_denied() {
    on_host_thread(|| {
        deny_io_uring_setup();
        let backend = Backend::Uring { entries: 8 };
        let rt = OsRuntime::try_new(std::env::temp_dir(), backend)
            .or_else(|_| OsRuntime::try_new(std::env::temp_dir(), Backend::Pool { threads: 1 }));
        assert!(rt.is_ok(), "the pool is never refused");
    });
}

#[test]
#[should_panic(expected = "faults are injected only beneath the pool backend")]
fn faults_refuse_the_uring_backend() {
    OsRuntime::with_faults(
        std::env::temp_dir(),
        Backend::Uring { entries: 8 },
        Faults::default(),
    );
}

#[test]
fn both_backends_report_a_device_with_no_space_as_full() {
    use std::io;

    use crate::runtime::IoError;

    for kind in [io::ErrorKind::StorageFull, io::ErrorKind::QuotaExceeded] {
        assert_eq!(
            IoError::Full,
            super::files::error(&io::Error::from(kind)),
            "{kind:?}"
        );
    }
    for errno in [libc::ENOSPC, libc::EDQUOT] {
        assert_eq!(IoError::Full, super::uring::failure(errno), "{errno}");
    }
    assert_eq!(IoError::Failed, super::uring::failure(libc::EIO));
}
