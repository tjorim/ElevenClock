//! Worker/helper lifetime tests against a real child process (R08).
//!
//! These run on every host. They check the properties the issue asks to be
//! *proved*, not assumed: old-generation results are rejected, a fresh request
//! recovers, deadlines discard partial output, helper exit is observed before a
//! replacement starts, and repeated failures neither grow threads, handles,
//! memory or temporary files nor leave helper processes behind.
//!
//! The helper is this same test executable re-launched with `--fixture`, a
//! stand-in process (the test target uses its own `main`, so the fixture's stdout
//! is exactly the report). It exercises the production supervisor, job isolation
//! (on Windows) and wire parsing, but it is a controllable fixture: it does **not**
//! make a UI Automation call, so it is never evidence about Explorer or a real
//! provider.
use clock_core::ShellGeneration;
use clock_windows::{
    runtime::{DiscoveryFailure, DiscoveryReport},
    taskbar::worker::{DiscoveryWorker, HelperSpec, SpecFactory, Stats},
};
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

fn fixture() -> PathBuf {
    std::env::current_exe().unwrap()
}

fn report(generation: u64, taskbars: u64) -> String {
    let taskbars: Vec<_> = (1..=taskbars)
        .map(|id| {
            let left = (id as i64 - 1) * -1920;
            format!(
                r#"{{"id":{id},"hwnd":{hwnd},"taskbar":[{left},1032,1920,48],"monitor_id":{id},"monitor":[{left},0,1920,1080],"dpi":96,"primary":{primary},"visible":true,"clock":{{"rect":[{clock},1032,100,48],"adjacent":[[{edge},1032,20,48]],"exclusions_complete":true}}}}"#,
                hwnd = 0x10000 + id,
                primary = id == 1,
                clock = left + 1800,
                edge = left + 1900,
            )
        })
        .collect();
    format!(
        r#"{{"schema":2,"generation":{generation},"shell":{{"pid":4242,"started":133000000000000000}},"taskbars":[{}]}}"#,
        taskbars.join(",")
    )
}

/// The stand-in helper. Modes: report, wrong-generation, slow, report-then-stall
/// (a complete-looking report followed by a hang: a deadline must discard it),
/// stall, exit, garbage, huge.
fn run_fixture(args: &[String]) -> ! {
    let mode = args.first().map_or("", String::as_str);
    let generation: u64 = args.get(1).and_then(|g| g.parse().ok()).unwrap_or(0);
    let extra: u64 = args.get(2).and_then(|g| g.parse().ok()).unwrap_or(0);
    if let Some(path) = std::env::var_os("ELEVENCLOCK_RS_FIXTURE_PID") {
        let _ = std::fs::write(path, std::process::id().to_string());
    }
    let out = |text: &str| {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
    };
    match mode {
        "report" => out(&report(generation, extra.max(1))),
        "wrong-generation" => out(&report(generation + 1, 1)),
        "slow" => {
            std::thread::sleep(Duration::from_millis(extra));
            out(&report(generation, 1));
        }
        "report-then-stall" => {
            out(&report(generation, 1));
            std::thread::sleep(Duration::from_secs(120));
        }
        "stall" => std::thread::sleep(Duration::from_secs(120)),
        "exit" => std::process::exit(i32::try_from(extra).unwrap_or(1)),
        "garbage" => out("this is not json"),
        "huge" => out(&" ".repeat(300 * 1024)),
        _ => std::process::exit(64),
    }
    std::process::exit(0)
}

/// Parent-loss fixture: owns a worker whose helper stalls, reports the helper's
/// pid and then waits to be killed abruptly by the test.
#[cfg(windows)]
fn run_parent(args: &[String]) -> ! {
    let pid_file = PathBuf::from(&args[0]);
    let worker = DiscoveryWorker::spawn(
        factory(|_| ("stall", 0), Some(pid_file)),
        Duration::from_secs(120),
        Box::new(|| {}),
    )
    .unwrap();
    worker.request(ShellGeneration(1));
    std::thread::sleep(Duration::from_secs(300));
    std::process::exit(0)
}
/// `plan` maps a generation to the fixture mode and extra argument.
fn factory(
    plan: impl Fn(u64) -> (&'static str, u64) + Send + Sync + 'static,
    pid_file: Option<PathBuf>,
) -> SpecFactory {
    let program = fixture();
    Box::new(move |generation: ShellGeneration| {
        let (mode, extra) = plan(generation.0);
        let mut env = Vec::new();
        if let Some(path) = &pid_file {
            env.push((
                "ELEVENCLOCK_RS_FIXTURE_PID".into(),
                path.clone().into_os_string(),
            ));
        }
        HelperSpec {
            program: program.clone(),
            args: vec![
                "--fixture".into(),
                mode.into(),
                generation.0.to_string().into(),
                extra.to_string().into(),
            ],
            env,
            handshake: b"r08-go:native\n".to_vec(),
        }
    })
}

struct Harness {
    worker: DiscoveryWorker,
    notified: Arc<AtomicU64>,
}
fn harness(
    plan: impl Fn(u64) -> (&'static str, u64) + Send + Sync + 'static,
    deadline: Duration,
    pid_file: Option<PathBuf>,
) -> Harness {
    let notified = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&notified);
    let worker = DiscoveryWorker::spawn(
        factory(plan, pid_file),
        deadline,
        Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap();
    Harness { worker, notified }
}
impl Harness {
    fn wait_for_reports(&self, count: usize) -> Vec<DiscoveryReport> {
        let started = Instant::now();
        let mut reports = Vec::new();
        while reports.len() < count {
            reports.extend(self.worker.take_reports());
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "timed out waiting for {count} reports; have {}",
                reports.len()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        reports
    }
    fn stat(&self, counter: fn(&Stats) -> &AtomicU64) -> u64 {
        Stats::get(counter(self.worker.stats()))
    }
}

fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(windows)]
    {
        use windows::Win32::{
            Foundation::{CloseHandle, STILL_ACTIVE},
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        // SAFETY: query-only handle, closed on every path.
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let mut code = 0u32;
            let alive =
                GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
            let _ = CloseHandle(handle);
            alive
        }
    }
}
fn wait_for_file(path: &PathBuf) -> u32 {
    let started = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && let Ok(pid) = text.trim().parse()
        {
            return pid;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "helper never started"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn leftover_outputs() -> usize {
    let prefix = format!("elevenclock-rs-r08-{}-", std::process::id());
    std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .count()
}

#[derive(Debug, Clone, Copy)]
struct Usage {
    threads: usize,
    handles: usize,
    memory_kib: u64,
}
fn usage() -> Usage {
    #[cfg(target_os = "linux")]
    {
        let count = |dir: &str| std::fs::read_dir(dir).unwrap().count();
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let rss = status
            .lines()
            .find_map(|l| l.strip_prefix("VmRSS:"))
            .and_then(|v| v.split_whitespace().next()?.parse().ok())
            .unwrap_or(0);
        Usage {
            threads: count("/proc/self/task"),
            handles: count("/proc/self/fd"),
            memory_kib: rss,
        }
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::{
            Diagnostics::ToolHelp::*,
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{GetCurrentProcess, GetCurrentProcessId, GetProcessHandleCount},
        };
        // SAFETY: documented query APIs with correctly sized, live buffers; the
        // snapshot handle is closed.
        unsafe {
            let mut handles = 0u32;
            GetProcessHandleCount(GetCurrentProcess(), &mut handles).unwrap();
            let mut counters = PROCESS_MEMORY_COUNTERS {
                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                ..Default::default()
            };
            GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb).unwrap();
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0).unwrap();
            let mut entry = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            let pid = GetCurrentProcessId();
            let mut threads = 0;
            let mut more = Thread32First(snapshot, &mut entry).is_ok();
            while more {
                if entry.th32OwnerProcessID == pid {
                    threads += 1;
                }
                more = Thread32Next(snapshot, &mut entry).is_ok();
            }
            let _ = windows::Win32::Foundation::CloseHandle(snapshot);
            Usage {
                threads,
                handles: handles as usize,
                memory_kib: counters.WorkingSetSize as u64 / 1024,
            }
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Usage {
            threads: 0,
            handles: 0,
            memory_kib: 0,
        }
    }
}

fn a_current_generation_report_is_published_with_native_handles_kept_private() {
    let h = harness(|_| ("report", 2), Duration::from_secs(10), None);
    h.worker.request(ShellGeneration(4));
    let reports = h.wait_for_reports(1);
    assert_eq!(reports[0].generation, ShellGeneration(4));
    let snapshots = reports[0].outcome.as_ref().expect("valid report");
    assert_eq!(snapshots.len(), 2);
    // The fixture's clock record is geometrically valid, so it is accepted.
    assert!(snapshots.iter().all(|s| s.clock_rect.is_some()));
    assert_eq!(h.worker.handle_for(snapshots[1].key), Some(0x10002));
    assert!(h.notified.load(Ordering::SeqCst) >= 1);
    assert_eq!(h.stat(|s| &s.spawned), 1);
    assert_eq!(h.stat(|s| &s.exited_observed), 1);
}

fn a_report_for_another_generation_is_rejected_not_published_as_current() {
    let h = harness(|_| ("wrong-generation", 0), Duration::from_secs(10), None);
    h.worker.request(ShellGeneration(9));
    let reports = h.wait_for_reports(1);
    assert!(matches!(
        &reports[0].outcome,
        Err(DiscoveryFailure::Rejected(_))
    ));
    assert!(
        h.worker
            .handle_for(clock_core::TaskbarKey {
                generation: ShellGeneration(10),
                id: clock_core::TaskbarId(1)
            })
            .is_none()
    );
    assert_eq!(h.stat(|s| &s.rejected), 1);
}

fn malformed_oversized_and_failing_helpers_fail_closed() {
    for (mode, extra) in [("garbage", 0), ("huge", 0), ("exit", 3)] {
        let h = harness(move |_| (mode, extra), Duration::from_secs(10), None);
        h.worker.request(ShellGeneration(1));
        let reports = h.wait_for_reports(1);
        match (&reports[0].outcome, mode) {
            (Err(DiscoveryFailure::HelperExit(Some(3))), "exit") => {}
            (Err(DiscoveryFailure::Rejected(_)), "garbage" | "huge") => {}
            (other, _) => panic!("{mode}: unexpected {other:?}"),
        }
    }
}

fn a_deadline_discards_even_a_complete_looking_partial_report_and_reaps_the_helper() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let h = harness(
        |_| ("report-then-stall", 0),
        Duration::from_millis(400),
        Some(pid_file.clone()),
    );
    h.worker.request(ShellGeneration(2));
    let pid = wait_for_file(&pid_file);
    let reports = h.wait_for_reports(1);
    assert_eq!(
        reports[0].outcome.as_ref().unwrap_err(),
        &DiscoveryFailure::Deadline
    );
    // Exit was observed before the result was published.
    assert!(!process_alive(pid), "helper {pid} survived its deadline");
    assert_eq!(h.stat(|s| &s.deadlines), 1);
    assert_eq!(h.stat(|s| &s.spawned), h.stat(|s| &s.exited_observed));
}

fn a_new_generation_terminates_the_running_helper_and_only_the_newest_publishes() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    // Generation 1 hangs; generation 2 answers promptly.
    let h = harness(
        |g| if g == 1 { ("stall", 0) } else { ("report", 1) },
        Duration::from_secs(30),
        Some(pid_file.clone()),
    );
    h.worker.request(ShellGeneration(1));
    let stalled = wait_for_file(&pid_file);
    std::fs::remove_file(&pid_file).unwrap();
    h.worker.request(ShellGeneration(2));
    let reports = h.wait_for_reports(1);
    assert_eq!(reports[0].generation, ShellGeneration(2));
    assert!(reports[0].outcome.is_ok());
    assert!(
        !process_alive(stalled),
        "stale helper must be gone before the replacement result"
    );
    assert_eq!(h.stat(|s| &s.cancelled), 1);
    assert_eq!(h.stat(|s| &s.spawned), 2);
    assert_eq!(h.stat(|s| &s.exited_observed), 2);
    // Nothing from generation 1 appears later.
    std::thread::sleep(Duration::from_millis(100));
    assert!(h.worker.take_reports().is_empty());
}

fn a_burst_of_requests_coalesces_to_one_helper_for_the_newest_generation() {
    let h = harness(|_| ("slow", 300), Duration::from_secs(10), None);
    for generation in 1..=20 {
        h.worker.request(ShellGeneration(generation));
    }
    let reports = h.wait_for_reports(1);
    assert_eq!(reports[0].generation, ShellGeneration(20));
    std::thread::sleep(Duration::from_millis(100));
    assert!(h.worker.take_reports().is_empty());
    assert!(
        h.stat(|s| &s.spawned) <= 2,
        "at most the first and the newest ran"
    );
}

fn shutdown_terminates_a_running_helper_and_joins_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let mut h = harness(
        |_| ("stall", 0),
        Duration::from_secs(60),
        Some(pid_file.clone()),
    );
    h.worker.request(ShellGeneration(1));
    let pid = wait_for_file(&pid_file);
    assert!(h.worker.shutdown(Duration::from_secs(5)));
    assert!(!h.worker.is_running());
    assert!(!process_alive(pid));
    // Requests after shutdown are inert and spawn nothing.
    h.worker.request(ShellGeneration(2));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(h.stat(|s| &s.spawned), 1);
}

fn repeated_failures_timeouts_and_recoveries_do_not_accumulate_resources() {
    let h = harness(
        |g| match g % 5 {
            0 => ("stall", 0),   // deadline
            1 => ("exit", 7),    // crash
            2 => ("garbage", 0), // malformed
            3 => ("wrong-generation", 0),
            _ => ("report", 2), // success
        },
        Duration::from_millis(150),
        None,
    );
    // Warm up so one-time allocations are not counted as growth.
    for generation in 1..=5 {
        h.worker.request(ShellGeneration(generation));
        h.wait_for_reports(1);
    }
    let before = usage();
    let rounds = 60u64;
    for generation in 6..6 + rounds {
        h.worker.request(ShellGeneration(generation));
        let reports = h.wait_for_reports(1);
        assert_eq!(reports[0].generation, ShellGeneration(generation));
    }
    let after = usage();
    let spawned = h.stat(|s| &s.spawned);
    assert_eq!(spawned, 5 + rounds, "exactly one helper per request");
    assert_eq!(
        spawned,
        h.stat(|s| &s.exited_observed),
        "every helper's exit was observed"
    );
    assert_eq!(after.threads, before.threads, "{before:?} -> {after:?}");
    assert!(
        after.handles <= before.handles + 2,
        "handles grew: {before:?} -> {after:?}"
    );
    assert!(
        after.memory_kib <= before.memory_kib + 16 * 1024,
        "memory grew: {before:?} -> {after:?}"
    );
    assert_eq!(leftover_outputs(), 0, "temporary outputs are removed");
}

#[cfg(windows)]
fn parent_loss_removes_the_helper_job() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let mut parent = std::process::Command::new(fixture())
        .args(["--parent", pid_file.to_str().unwrap()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let helper = wait_for_file(&pid_file);
    assert!(process_alive(helper));
    // Abrupt termination: no destructor, no shutdown, no cooperation.
    parent.kill().unwrap();
    parent.wait().unwrap();
    let started = Instant::now();
    while process_alive(helper) {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "helper {helper} outlived its parent"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

type TestFn = fn();
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--fixture") => run_fixture(&args[1..]),
        #[cfg(windows)]
        Some("--parent") => run_parent(&args[1..]),
        _ => {}
    }
    let filters: Vec<_> = args.iter().filter(|a| !a.starts_with("--")).collect();
    #[allow(unused_mut)]
    let mut tests: Vec<(&str, TestFn)> = vec![
        (
            "a_current_generation_report_is_published_with_native_handles_kept_private",
            a_current_generation_report_is_published_with_native_handles_kept_private,
        ),
        (
            "a_report_for_another_generation_is_rejected_not_published_as_current",
            a_report_for_another_generation_is_rejected_not_published_as_current,
        ),
        (
            "malformed_oversized_and_failing_helpers_fail_closed",
            malformed_oversized_and_failing_helpers_fail_closed,
        ),
        (
            "a_deadline_discards_even_a_complete_looking_partial_report_and_reaps_the_helper",
            a_deadline_discards_even_a_complete_looking_partial_report_and_reaps_the_helper,
        ),
        (
            "a_new_generation_terminates_the_running_helper_and_only_the_newest_publishes",
            a_new_generation_terminates_the_running_helper_and_only_the_newest_publishes,
        ),
        (
            "a_burst_of_requests_coalesces_to_one_helper_for_the_newest_generation",
            a_burst_of_requests_coalesces_to_one_helper_for_the_newest_generation,
        ),
        (
            "shutdown_terminates_a_running_helper_and_joins_the_worker",
            shutdown_terminates_a_running_helper_and_joins_the_worker,
        ),
        (
            "repeated_failures_timeouts_and_recoveries_do_not_accumulate_resources",
            repeated_failures_timeouts_and_recoveries_do_not_accumulate_resources,
        ),
    ];
    #[cfg(windows)]
    tests.push((
        "parent_loss_removes_the_helper_job",
        parent_loss_removes_the_helper_job,
    ));
    let mut failed = Vec::new();
    for (name, test) in tests {
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        print!("test {name} ... ");
        let _ = std::io::stdout().flush();
        match std::panic::catch_unwind(test) {
            Ok(()) => println!("ok"),
            Err(_) => {
                println!("FAILED");
                failed.push(name);
            }
        }
    }
    if !failed.is_empty() {
        println!("\nfailed: {failed:?}");
        std::process::exit(1);
    }
}
