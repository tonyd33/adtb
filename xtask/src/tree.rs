use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// The rounds of `ROUND_MS` that `kill` waits for the processes it killed to exit.
const ROUNDS: usize = 100;
const ROUND_MS: libc::c_int = 100;

/// A handle on one process. It names that process after it exits, so a reused pid cannot
/// redirect it.
pub struct Pidfd {
    pub pid: u32,
    fd: OwnedFd,
}

impl Pidfd {
    /// Returns a handle on the process `pid` names now.
    pub fn open(pid: u32) -> Result<Self, String> {
        let raw = libc::pid_t::try_from(pid).map_err(|e| format!("pid {pid}: {e}"))?;
        // SAFETY: pidfd_open takes no pointers.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, raw, 0) };
        if fd < 0 {
            return Err(format!("pidfd_open {pid}: {}", io::Error::last_os_error()));
        }
        let fd = libc::c_int::try_from(fd).map_err(|e| format!("pidfd_open {pid}: {e}"))?;
        // SAFETY: pidfd_open returned a new descriptor that nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self { pid, fd })
    }

    /// Send `signal` to the process. A process that has exited takes none.
    fn signal(&self, signal: libc::c_int) {
        // SAFETY: a null info pointer asks for the info of a plain kill.
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
    }

    /// Returns whether the process has exited, waiting up to `timeout_ms`, or without end at -1.
    pub fn exited(&self, timeout_ms: libc::c_int) -> Result<bool, String> {
        let mut poll = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            // SAFETY: the pointer names one `pollfd`, as the count says.
            let ready = unsafe { libc::poll(&raw mut poll, 1, timeout_ms) };
            if ready >= 0 {
                return Ok(ready > 0);
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(format!("polling process {}: {e}", self.pid));
            }
        }
    }
}

/// Stop the process `root` holds and every process under it until no new one appears, kill them
/// all, and wait for them to exit. Returns the pids of those still running after the wait.
///
/// Preconditions:
/// - `root` is a child of this process that has not been reaped.
pub fn kill(root: &Pidfd) -> Result<Vec<u32>, String> {
    root.signal(libc::SIGSTOP);
    let mut under: Vec<Pidfd> = Vec::new();
    loop {
        let fresh = children(root, &under)?;
        if fresh.is_empty() {
            break;
        }
        for process in &fresh {
            process.signal(libc::SIGSTOP);
        }
        under.extend(fresh);
    }
    let mut left: Vec<&Pidfd> = std::iter::once(root).chain(&under).collect();
    for process in &left {
        process.signal(libc::SIGKILL);
    }
    for _ in 0..ROUNDS {
        let mut running = Vec::new();
        for process in left {
            if !process.exited(ROUND_MS)? {
                running.push(process);
            }
        }
        left = running;
        if left.is_empty() {
            break;
        }
    }
    Ok(left.into_iter().map(|process| process.pid).collect())
}

/// Returns a handle on each process whose parent is `root` or one of `under` and which is not
/// among them. The process and its parent were both running after the parent was read, so the
/// parent's pid named that parent.
fn children(root: &Pidfd, under: &[Pidfd]) -> Result<Vec<Pidfd>, String> {
    let tree: Vec<&Pidfd> = std::iter::once(root).chain(under).collect();
    let mut fresh = Vec::new();
    for pid in pids()? {
        if tree.iter().any(|process| process.pid == pid) {
            continue;
        }
        let Ok(process) = Pidfd::open(pid) else {
            continue;
        };
        let Some((_, ppid)) = stat(pid) else {
            continue;
        };
        let Some(parent) = tree.iter().find(|process| process.pid == ppid) else {
            continue;
        };
        if !parent.exited(0)? && !process.exited(0)? {
            fresh.push(process);
        }
    }
    Ok(fresh)
}

/// Returns the pid and command name of each running process whose parent is this process.
pub fn adopted() -> Result<Vec<(u32, String)>, String> {
    let me = std::process::id();
    Ok(pids()?
        .into_iter()
        .filter_map(|pid| {
            let (state, ppid) = stat(pid)?;
            (ppid == me && state != 'Z').then(|| {
                let name = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
                (pid, name.trim_end().to_owned())
            })
        })
        .collect())
}

fn pids() -> Result<Vec<u32>, String> {
    let entries = fs::read_dir("/proc").map_err(|e| format!("reading /proc: {e}"))?;
    Ok(entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect())
}

/// Returns the state and the parent pid of process `pid`, or `None` when it has gone.
fn stat(pid: u32) -> Option<(char, u32)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let mut fields = stat.rsplit_once(')')?.1.split_whitespace();
    let state = fields.next()?.chars().next()?;
    Some((state, fields.next()?.parse().ok()?))
}

/// Make this process the reaper of every orphan under it.
pub fn adopt_orphans() -> Result<(), String> {
    // SAFETY: PR_SET_CHILD_SUBREAPER takes one integer argument.
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) } != 0 {
        return Err(format!(
            "prctl PR_SET_CHILD_SUBREAPER: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}
