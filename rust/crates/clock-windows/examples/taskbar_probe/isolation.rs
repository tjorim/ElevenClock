//! Development-only one-process supervisor, with a real stalled-process test.
use std::{
    io,
    process::{Child, ExitStatus},
    thread,
    time::{Duration, Instant},
};

pub enum Completion {
    Exited(ExitStatus),
    Deadline { teardown_micros: u128 },
}

/// Windows kill-on-close ownership also handles a supervisor crash. No process
/// handle or job is returned through a discovery result channel.
#[cfg(windows)]
pub struct HelperJob(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl HelperJob {
    pub fn attach(child: &Child) -> windows::core::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, System::JobObjects::*};
        // SAFETY: unnamed, non-inherited owned job; correctly sized structure.
        // The borrowed child handle remains live for the synchronous assignment.
        unsafe {
            let job = Self(CreateJobObjectW(None, None)?);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            limits.BasicLimitInformation.ActiveProcessLimit = 1;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )?;
            AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle()))?;
            Ok(job)
        }
    }
}

#[cfg(windows)]
impl Drop for HelperJob {
    fn drop(&mut self) {
        // SAFETY: exactly one close of our owned job. Kill-on-close ensures no
        // stalled helper survives loss of the supervisor's last job handle.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// No pipe reads while the child is running. After a deadline, terminate and
/// observe exit before another probe may start. A teardown error is fatal for
/// this supervisor; never retry by spawning another helper.
pub fn supervise(child: &mut Child, deadline: Duration) -> io::Result<Completion> {
    let start = Instant::now();
    loop {
        if start.elapsed() >= deadline {
            break;
        }
        if let Some(status) = child.try_wait()? {
            return Ok(Completion::Exited(status));
        }
        thread::sleep(Duration::from_millis(10));
    }
    let teardown = Instant::now();
    if child.try_wait()?.is_none()
        && let Err(error) = child.kill()
        && child.try_wait()?.is_none()
    {
        return Err(error);
    }
    while teardown.elapsed() < Duration::from_secs(1) {
        if child.try_wait()?.is_some() {
            return Ok(Completion::Deadline {
                teardown_micros: teardown.elapsed().as_micros(),
            });
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "Helper exit not observed; supervisor must stop, not respawn",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    #[ignore = "subprocess fixture invoked only by deadline test"]
    fn stalled_helper() {
        if std::env::var_os("ELEVENCLOCK_TEST_STALL").is_some() {
            if let Some(marker) = std::env::var_os("ELEVENCLOCK_TEST_MARKER") {
                std::fs::write(marker, "started").unwrap();
            }
            thread::sleep(Duration::from_secs(30));
        }
    }

    #[test]
    fn kills_and_reaps_a_stalled_helper_before_replacement() {
        let start = Instant::now();
        let marker = std::env::temp_dir().join(format!(
            "elevenclock-stall-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "isolation::tests::stalled_helper", "--ignored"])
            .env("ELEVENCLOCK_TEST_STALL", "1")
            .env("ELEVENCLOCK_TEST_MARKER", &marker)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        #[cfg(windows)]
        let _job = HelperJob::attach(&child).unwrap();
        while !marker.exists() && start.elapsed() < Duration::from_secs(3) {
            assert!(child.try_wait().unwrap().is_none());
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            marker.exists(),
            "Child must enter its stalled operation before the deadline test"
        );
        match supervise(&mut child, Duration::from_millis(100)).unwrap() {
            Completion::Deadline { teardown_micros } => assert!(teardown_micros < 1_000_000),
            Completion::Exited(status) => panic!("Stall fixture unexpectedly exited: {status}"),
        }
        assert!(child.try_wait().unwrap().is_some());
        std::fs::remove_file(marker).unwrap();
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn preserves_normal_helper_exit_status() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "isolation::tests::stalled_helper", "--ignored"])
            .env_remove("ELEVENCLOCK_TEST_STALL")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        #[cfg(windows)]
        let _job = HelperJob::attach(&child).unwrap();
        match supervise(&mut child, Duration::from_secs(3)).unwrap() {
            Completion::Exited(status) => assert!(status.success()),
            Completion::Deadline { .. } => panic!("Normal fixture timed out"),
        }
    }

    #[test]
    fn rejects_completion_observed_after_deadline() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "isolation::tests::stalled_helper", "--ignored"])
            .env_remove("ELEVENCLOCK_TEST_STALL")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert!(matches!(
            supervise(&mut child, Duration::ZERO).unwrap(),
            Completion::Deadline { .. }
        ));
    }
}
