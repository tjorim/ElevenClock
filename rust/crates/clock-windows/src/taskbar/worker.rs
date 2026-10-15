//! Bounded, supervised discovery worker (R08).
//!
//! One persistent worker thread runs at most one helper process at a time. A
//! helper is the only place blocking shell/UIA calls happen, so nothing the UI
//! thread does can wait on Explorer or a UI Automation provider, and a stalled
//! provider is contained by terminating a *process*, never by pretending COM calls
//! can be cancelled (see ADR 0001 and ADR 0002).
//!
//! Lifetime rules, each covered by tests below:
//! * the helper is assigned to a kill-on-close, single-process job *before* it is
//!   allowed to start native work, so parent loss removes it;
//! * a result is accepted only from a helper whose exit was observed, and only if
//!   it is still the current generation when published;
//! * a deadline, cancellation or superseding request terminates the helper and
//!   waits for its exit before any replacement starts;
//! * if exit cannot be observed the worker stops permanently and reports
//!   `TeardownFailed`; it never respawns while a helper might still be alive;
//! * thread count, process count and temporary files stay constant however many
//!   requests fail.
use super::{ProbeQueue, protocol};
use crate::runtime::{DiscoveryFailure, DiscoveryReport};
use clock_core::{ShellGeneration, TaskbarKey};
use std::{
    collections::{HashMap, VecDeque},
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Result deadline for one helper, including serialization and exit.
pub const DEADLINE: Duration = Duration::from_secs(5);
/// How long exit may take after termination before the worker stops for good.
pub const TEARDOWN_LIMIT: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(10);

/// How to start one helper. Production uses the app's own executable with a
/// private marker; tests substitute a fixture process.
#[derive(Clone, Debug)]
pub struct HelperSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    /// Bytes written to the helper's stdin once it is inside its job. The helper
    /// does no native work before reading them.
    pub handshake: Vec<u8>,
}

pub type SpecFactory = Box<dyn Fn(ShellGeneration) -> HelperSpec + Send + Sync>;

/// Production helper: this executable, a private role marker, then a handshake
/// that selects UIA discovery.
pub fn same_executable() -> std::io::Result<SpecFactory> {
    let program = std::env::current_exe()?;
    Ok(Box::new(move |generation| HelperSpec {
        program: program.clone(),
        args: vec![
            crate::instance::HELPER_MARKER.into(),
            generation.0.to_string().into(),
        ],
        env: Vec::new(),
        handshake: b"r08-go:uia\n".to_vec(),
    }))
}

/// Counters that make "no accumulation" measurable by tests and diagnostics.
#[derive(Debug, Default)]
pub struct Stats {
    pub spawned: AtomicU64,
    pub exited_observed: AtomicU64,
    pub deadlines: AtomicU64,
    pub cancelled: AtomicU64,
    pub rejected: AtomicU64,
    pub published: AtomicU64,
    pub discarded_stale: AtomicU64,
}
impl Stats {
    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }
}

/// Windows job: kill-on-close and a one-process limit.
#[cfg(windows)]
pub struct HelperJob(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl HelperJob {
    pub fn attach(child: &Child) -> windows::core::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, System::JobObjects::*};
        // SAFETY: unnamed, non-inherited owned job and a correctly sized limits
        // structure. The borrowed child handle stays live for the synchronous call.
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
        // SAFETY: one close of our owned job; kill-on-close removes any survivor.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

struct TempOutput(PathBuf);
impl Drop for TempOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
static OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum RunError {
    /// Stop requested or superseded; the helper was terminated and reaped.
    Cancelled,
    Failure(DiscoveryFailure),
}
impl From<DiscoveryFailure> for RunError {
    fn from(failure: DiscoveryFailure) -> Self {
        Self::Failure(failure)
    }
}

/// Terminate and observe exit. `Err` means exit was not observed: fatal.
fn terminate(child: &mut Child) -> Result<(), DiscoveryFailure> {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return Ok(());
    }
    let _ = child.kill();
    let started = Instant::now();
    while started.elapsed() < TEARDOWN_LIMIT {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Err(DiscoveryFailure::TeardownFailed)
}

/// Run one helper to completion and return its raw output bytes. Reads output
/// only after observed exit.
pub fn run_helper(
    spec: &HelperSpec,
    deadline: Duration,
    cancelled: &dyn Fn() -> bool,
    stats: &Stats,
) -> Result<Vec<u8>, RunError> {
    let unavailable = |what: &str, error: &dyn std::fmt::Display| {
        DiscoveryFailure::Unavailable(format!("{what}: {error}"))
    };
    let path = std::env::temp_dir().join(format!(
        "mainspring-r08-{}-{}.json",
        std::process::id(),
        OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| unavailable("output file", &e))?;
    let _output = TempOutput(path.clone());

    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::from(file))
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().map_err(|e| unavailable("spawn", &e))?;
    stats.spawned.fetch_add(1, Ordering::Relaxed);

    // Isolation first: no native work happens before the job owns the process.
    #[cfg(windows)]
    let _job = match HelperJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            // Never retry by spawning more helpers: remove this one and report.
            terminate(&mut child)?;
            stats.exited_observed.fetch_add(1, Ordering::Relaxed);
            return Err(unavailable("job isolation", &error).into());
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        // A helper that already exited makes this fail; supervision reports it.
        let _ = stdin.write_all(&spec.handshake);
    }

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                terminate(&mut child)?;
                return Err(unavailable("wait", &error).into());
            }
        }
        if cancelled() {
            terminate(&mut child)?;
            stats.exited_observed.fetch_add(1, Ordering::Relaxed);
            stats.cancelled.fetch_add(1, Ordering::Relaxed);
            return Err(RunError::Cancelled);
        }
        if started.elapsed() >= deadline {
            terminate(&mut child)?;
            stats.exited_observed.fetch_add(1, Ordering::Relaxed);
            stats.deadlines.fetch_add(1, Ordering::Relaxed);
            return Err(DiscoveryFailure::Deadline.into());
        }
        std::thread::sleep(POLL);
    };
    stats.exited_observed.fetch_add(1, Ordering::Relaxed);
    if !status.success() {
        return Err(DiscoveryFailure::HelperExit(status.code()).into());
    }
    let length = fs::metadata(&path)
        .map_err(|e| unavailable("output size", &e))?
        .len();
    if length > protocol::MAX_OUTPUT_BYTES {
        return Err(DiscoveryFailure::Rejected("oversized output".into()).into());
    }
    Ok(fs::read(&path).map_err(|e| unavailable("output read", &e))?)
}

struct Shared {
    state: Mutex<WorkerState>,
    wake: Condvar,
    stats: Stats,
}
#[derive(Default)]
struct WorkerState {
    queue: ProbeQueue,
    latest: Option<ShellGeneration>,
    stop: bool,
    fatal: bool,
    reports: VecDeque<DiscoveryReport>,
    handles: HashMap<TaskbarKey, u64>,
}
fn lock(shared: &Shared) -> MutexGuard<'_, WorkerState> {
    // A poisoned lock only means a panic elsewhere; the data stays consistent
    // because every critical section here is a short, non-panicking update.
    shared.state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Called after a report is queued, from the worker thread (post a window message).
pub type Notify = Box<dyn Fn() + Send + Sync>;

pub struct DiscoveryWorker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl DiscoveryWorker {
    pub fn spawn(
        factory: SpecFactory,
        deadline: Duration,
        notify: Notify,
    ) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(WorkerState::default()),
            wake: Condvar::new(),
            stats: Stats::default(),
        });
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("mainspring-discovery".into())
            .spawn(move || worker_loop(&worker, &factory, deadline, &notify))?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    /// Queue discovery for `generation`. A running helper for an older generation
    /// is terminated; only the newest request survives (coalesced).
    pub fn request(&self, generation: ShellGeneration) {
        let mut state = lock(&self.shared);
        if state.stop || state.fatal {
            return;
        }
        state.latest = Some(generation);
        state.queue.request(generation);
        drop(state);
        self.shared.wake.notify_all();
    }

    pub fn take_reports(&self) -> Vec<DiscoveryReport> {
        lock(&self.shared).reports.drain(..).collect()
    }
    /// The helper's window handle for a taskbar of the latest generation. Native
    /// code must revalidate it before use.
    pub fn handle_for(&self, key: TaskbarKey) -> Option<u64> {
        lock(&self.shared).handles.get(&key).copied()
    }
    pub fn stats(&self) -> &Stats {
        &self.shared.stats
    }
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Stop the worker. A running helper is terminated and reaped by the worker
    /// thread; the wait here is bounded so a hung teardown cannot hold up exit
    /// (the kill-on-close job still removes the helper when this process ends).
    pub fn shutdown(&mut self, wait: Duration) -> bool {
        {
            let mut state = lock(&self.shared);
            state.stop = true;
        }
        self.shared.wake.notify_all();
        let Some(thread) = self.thread.take() else {
            return true;
        };
        let started = Instant::now();
        while !thread.is_finished() {
            if started.elapsed() >= wait {
                self.thread = Some(thread);
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = thread.join();
        true
    }
}
impl Drop for DiscoveryWorker {
    fn drop(&mut self) {
        self.shutdown(TEARDOWN_LIMIT * 2);
    }
}

fn worker_loop(shared: &Shared, factory: &SpecFactory, deadline: Duration, notify: &Notify) {
    loop {
        let generation = {
            let mut state = lock(shared);
            loop {
                if state.stop {
                    return;
                }
                if let Some(generation) = state.queue.start() {
                    break generation;
                }
                state = shared.wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        };
        let spec = factory(generation);
        let cancelled = || {
            let state = lock(shared);
            state.stop || state.latest != Some(generation)
        };
        let run = match run_helper(&spec, deadline, &cancelled, &shared.stats) {
            Ok(bytes) => match protocol::parse_report(&bytes, generation) {
                Ok(parsed) => Run::Parsed(parsed),
                Err(failure) => {
                    shared.stats.rejected.fetch_add(1, Ordering::Relaxed);
                    Run::Failed(failure)
                }
            },
            Err(RunError::Cancelled) => Run::Cancelled,
            Err(RunError::Failure(failure)) => Run::Failed(failure),
        };
        let fatal = matches!(run, Run::Failed(DiscoveryFailure::TeardownFailed));
        let published = {
            let mut state = lock(shared);
            // finish() runs only now: after observed exit, or after fatal teardown.
            let current = state.queue.finish(generation) && !state.stop;
            if fatal {
                state.fatal = true;
            }
            let report = match run {
                Run::Cancelled => None,
                _ if !current && !fatal => None,
                Run::Parsed(parsed) => {
                    state.handles = parsed.handles.into_iter().collect();
                    Some(DiscoveryReport {
                        generation,
                        shell: parsed.shell,
                        outcome: Ok(parsed.snapshots),
                    })
                }
                Run::Failed(failure) => Some(DiscoveryReport {
                    generation,
                    shell: None,
                    outcome: Err(failure),
                }),
            };
            match report {
                Some(report) => {
                    state.reports.push_back(report);
                    shared.stats.published.fetch_add(1, Ordering::Relaxed);
                    true
                }
                None => {
                    shared.stats.discarded_stale.fetch_add(1, Ordering::Relaxed);
                    false
                }
            }
        };
        if published {
            notify();
        }
        if fatal {
            return; // Never respawn while a helper may still be alive.
        }
    }
}

enum Run {
    Parsed(protocol::Parsed),
    Failed(DiscoveryFailure),
    Cancelled,
}
