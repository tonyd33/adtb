use super::*;
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitStatus, Stdio};

pub(super) struct Child {
    process: Option<std::process::Child>,
    deadline: Instant,
    log: File,
}

impl Child {
    pub(super) fn start(
        peer: Peer,
        index: usize,
        role: &str,
        root: &Path,
        deadline: Instant,
    ) -> Result<Self> {
        let mut command = adtb_heap_cap::test_child(std::env::current_exe()?, 1);
        command
            .args(["--exact", ENTRY, "--nocapture"])
            .env(ROLE, role)
            .env(
                "ADTB_CRASH_PEER",
                if peer == Peer::Adtb { "adtb" } else { "sqlite" },
            )
            .env("ADTB_CRASH_CASE", index.to_string())
            .env("ADTB_CRASH_ROOT", root)
            .stdout(Stdio::from(File::create_new(
                root.join(format!("{role}.stdout")),
            )?))
            .stderr(Stdio::from(File::create_new(
                root.join(format!("{role}.stderr")),
            )?));
        let log = File::create_new(root.join(format!("{role}.process")))?;
        let mut owned = Self {
            process: Some(command.spawn()?),
            deadline,
            log,
        };
        writeln!(
            owned.log,
            "pid={} parent_group={:?} parent_cap={}",
            owned.process.as_ref().ok_or("missing owned child")?.id(),
            std::env::var("NEXTEST_TEST_GROUP"),
            adtb_heap_cap::current_cap()
        )?;
        Ok(owned)
    }

    pub(super) fn connect(&mut self, listener: &UnixListener) -> Result<UnixStream> {
        loop {
            if now() >= self.deadline {
                return Err("connect deadline".into());
            }
            match listener.accept() {
                Ok((stream, _)) => return Ok(stream),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if let Some(status) = self
                        .process
                        .as_mut()
                        .ok_or("missing owned child")?
                        .try_wait()?
                    {
                        return Err(format!("child exited before connection: {status}").into());
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn reap(&mut self, deadline: Instant) -> Result<ExitStatus> {
        loop {
            if now() >= deadline {
                return Err("owned child reap deadline".into());
            }
            if let Some(status) = self
                .process
                .as_mut()
                .ok_or("missing owned child")?
                .try_wait()?
            {
                self.process = None;
                writeln!(self.log, "reaped={status}")?;
                return Ok(status);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn terminate(&mut self) -> Result<()> {
        if now() >= self.deadline {
            return Err("kill deadline".into());
        }
        let child = self.process.as_mut().ok_or("missing owned child")?;
        if let Some(status) = child.try_wait()? {
            return Err(format!("writer exited before owned kill: {status}").into());
        }
        child.kill()?;
        let status = self.reap(self.deadline)?;
        if status.signal() != Some(9) {
            return Err(format!("owned kill did not terminate writer: {status}").into());
        }
        Ok(())
    }

    pub(super) fn finish(&mut self) -> Result<()> {
        let status = self.reap(self.deadline)?;
        if !status.success() {
            return Err(format!("reopen verifier failed: {status}").into());
        }
        Ok(())
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        if let Some(child) = self.process.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.process = None;
                    return;
                }
                Ok(None) => {
                    if let Err(e) = child.kill() {
                        eprintln!("owned child cleanup kill: {e}");
                    }
                }
                Err(e) => eprintln!("owned child cleanup status: {e}"),
            }
            if let Err(e) = self.reap(now() + Duration::from_secs(2)) {
                eprintln!("owned child cleanup reap failed: {e}");
            }
        }
    }
}
