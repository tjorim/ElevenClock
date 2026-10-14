//! UI Automation lifetime and failure evidence (R08, #9). Windows only.
//!
//! What this target is, precisely:
//! * **Controlled provider** scenarios: a fixture process owns a real top-level
//!   window. In `hang` mode its UI thread never pumps messages, so genuine UIA/COM
//!   calls into it block; in other scenarios it is terminated while a call is
//!   outstanding, or a client retains elements across its death. These are real UIA
//!   client calls against a controllable provider, **not** Explorer, and are labelled
//!   as such in the evidence. A `sleep` before COM is never used.
//! * The production inventory code (`taskbar::uia::inventory`) runs under the
//!   production supervisor (`taskbar::worker::run_helper`).
//! * **Real Explorer** scenario (opt-in, `ELEVENCLOCK_UIA_EXPLORER=1`, because it
//!   restarts the user's shell): a client makes continuous UIA calls on the taskbar;
//!   Explorer is killed while a call is demonstrably outstanding; the call's
//!   HRESULT, the stale-object result and the fresh recovery are recorded.
//!
//! Every result is also written as JSON to `ELEVENCLOCK_EVIDENCE_DIR` when set.
//! One executable plays every role (runner, provider, clients) via private flags.
#[cfg(not(windows))]
fn main() {
    println!("uia_lifetime: Windows only (UI Automation); skipped");
}

#[cfg(windows)]
fn main() {
    imp::main();
}

#[cfg(windows)]
mod imp {
    use clock_windows::taskbar::{
        probe, uia,
        worker::{HelperSpec, RunError, Stats, run_helper},
    };
    use serde_json::{Value, json};
    use std::{
        fs::{self, File, OpenOptions},
        io::Write,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use windows::{
        Win32::{
            Foundation::HWND,
            System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
            UI::{
                Accessibility::{
                    CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Children,
                },
                WindowsAndMessaging::{
                    CreateWindowExW, DispatchMessageW, FindWindowW, MSG, PM_REMOVE, PeekMessageW,
                    TranslateMessage, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE,
                },
            },
        },
        core::w,
    };

    fn micros() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
    }
    fn hex(code: i32) -> String {
        format!("0x{:08X}", code as u32)
    }
    fn exe() -> PathBuf {
        std::env::current_exe().unwrap()
    }

    // ------------------------------------------------------------------ roles

    /// Fixture provider. `normal` pumps messages; `hang` creates its window and then
    /// never services it, so calls into the window block.
    fn run_provider(args: &[String]) -> ! {
        let (mode, file) = (args[0].as_str(), &args[1]);
        // SAFETY: creates one top-level STATIC window on this thread.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("elevenclock-uia-fixture"),
                WS_POPUP | WS_VISIBLE,
                40,
                40,
                240,
                40,
                None,
                None,
                None,
                None,
            )
        }
        .expect("fixture window");
        fs::write(file, (hwnd.0 as usize).to_string()).unwrap();
        if mode == "hang" {
            std::thread::sleep(Duration::from_secs(300));
            std::process::exit(0);
        }
        let started = Instant::now();
        let mut message = MSG::default();
        while started.elapsed() < Duration::from_secs(180) {
            // SAFETY: standard message pump for the window created above.
            unsafe {
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        std::process::exit(0)
    }

    /// Production inventory against one window; prints a JSON summary.
    fn run_inventory(args: &[String]) -> ! {
        let hwnd = HWND(args[0].parse::<usize>().unwrap() as *mut _);
        let inventory = uia::inventory(hwnd);
        let summary = json!({
            "elements": inventory.elements.len(),
            "failures": inventory.failures,
            "truncated": inventory.truncated,
            "longest_call_micros": inventory.longest_call_micros as u64,
            "calls": inventory.calls.len(),
        });
        println!("{summary}");
        std::process::exit(0)
    }

    struct Tracer {
        file: File,
        seq: u64,
    }
    impl Tracer {
        fn record(&mut self, value: Value) {
            // One write per line so the runner sees records as they happen.
            let _ = self.file.write_all(format!("{value}\n").as_bytes());
        }
        /// Log `begin`, run the call, log `end` with its HRESULT. A line with a begin
        /// and no end is a call that is still outstanding.
        fn op<T>(
            &mut self,
            name: &str,
            shell: u32,
            call: impl FnOnce() -> windows::core::Result<T>,
        ) -> Option<T> {
            self.seq += 1;
            let seq = self.seq;
            self.record(json!({"t": micros(), "event": "begin", "op": name, "seq": seq, "shell_pid": shell}));
            let result = call();
            let hresult = result.as_ref().err().map(|e| hex(e.code().0));
            self.record(json!({"t": micros(), "event": "end", "op": name, "seq": seq, "shell_pid": shell, "ok": result.is_ok(), "hresult": hresult}));
            result.ok()
        }
    }

    fn target(spec: &str) -> Option<HWND> {
        if spec == "taskbar" {
            // SAFETY: class-name lookup of a top-level window.
            unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }.ok()
        } else {
            spec.parse::<usize>().ok().map(|v| HWND(v as *mut _))
        }
    }

    /// Fine-grained UIA client. Arguments: TARGET LOG SECONDS [READY GO].
    /// With READY/GO it acquires elements, signals, waits for GO and then probes the
    /// *retained* elements (stale-object behaviour). Without them it loops, and after
    /// the first failure probes retained elements once, then recovers with fresh
    /// objects and stops after three clean passes.
    fn run_trace(args: &[String]) -> ! {
        let spec = args[0].as_str();
        let mut tracer = Tracer {
            file: OpenOptions::new()
                .create(true)
                .append(true)
                .open(&args[1])
                .unwrap(),
            seq: 0,
        };
        let seconds: u64 = args[2].parse().unwrap();
        let stale_files = args.get(3).zip(args.get(4));
        // SAFETY: this client owns its MTA apartment for the whole process.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED).ok();
        }
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let mut automation: Option<IUIAutomation> = None;
        let mut retained: Vec<IUIAutomationElement> = Vec::new();
        let mut failed = false;
        let mut clean_after_failure = 0;
        while Instant::now() < deadline {
            let shell = if spec == "taskbar" {
                probe::current_shell().map_or(0, |s| s.0)
            } else {
                0
            };
            let Some(hwnd) = target(spec) else {
                tracer.record(json!({"t": micros(), "event": "no-target", "shell_pid": shell}));
                std::thread::sleep(Duration::from_millis(100));
                continue;
            };
            if automation.is_none() {
                // SAFETY: creates the in-process UI Automation client.
                automation = tracer.op("CoCreateInstance", shell, || unsafe {
                    windows::Win32::System::Com::CoCreateInstance(
                        &CUIAutomation,
                        None,
                        windows::Win32::System::Com::CLSCTX_INPROC_SERVER,
                    )
                });
            }
            let Some(a) = automation.clone() else {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            };
            // SAFETY (all UIA calls below): interfaces stay on this thread; every call
            // is individually logged with its HRESULT; nothing is invoked.
            let pass = (|| -> Option<Vec<IUIAutomationElement>> {
                let root = tracer.op("ElementFromHandle", shell, || unsafe {
                    a.ElementFromHandle(hwnd)
                })?;
                let condition = tracer.op("CreateTrueCondition", shell, || unsafe {
                    a.CreateTrueCondition()
                })?;
                let mut elements = vec![root.clone()];
                if let Some(array) = tracer.op("FindAll", shell, || unsafe {
                    root.FindAll(TreeScope_Children, &condition)
                }) {
                    let length = unsafe { array.Length() }.unwrap_or(0).min(16);
                    for i in 0..length {
                        if let Some(child) =
                            tracer.op("GetElement", shell, || unsafe { array.GetElement(i) })
                        {
                            elements.push(child);
                        }
                    }
                }
                for element in &elements {
                    tracer.op("CurrentClassName", shell, || unsafe {
                        element.CurrentClassName()
                    })?;
                    tracer.op("CurrentAutomationId", shell, || unsafe {
                        element.CurrentAutomationId()
                    })?;
                    tracer.op("CurrentBoundingRectangle", shell, || unsafe {
                        element.CurrentBoundingRectangle()
                    })?;
                }
                Some(elements)
            })();
            match pass {
                Some(elements) => {
                    if retained.is_empty() && !failed {
                        retained = elements;
                        if let Some((ready, go)) = stale_files {
                            fs::write(ready, "ready").unwrap();
                            let wait = Instant::now();
                            while !Path::new(go).exists()
                                && wait.elapsed() < Duration::from_secs(60)
                            {
                                std::thread::sleep(Duration::from_millis(20));
                            }
                            tracer.record(json!({"t": micros(), "event": "go"}));
                            for element in &retained {
                                tracer.op("stale-CurrentClassName", shell, || unsafe {
                                    element.CurrentClassName()
                                });
                                tracer.op("stale-CurrentBoundingRectangle", shell, || unsafe {
                                    element.CurrentBoundingRectangle()
                                });
                            }
                            tracer.record(json!({"t": micros(), "event": "done"}));
                            std::process::exit(0);
                        }
                    }
                    if failed {
                        clean_after_failure += 1;
                        if clean_after_failure >= 3 {
                            tracer.record(
                                json!({"t": micros(), "event": "recovered", "shell_pid": shell}),
                            );
                            break;
                        }
                    }
                }
                None => {
                    failed = true;
                    clean_after_failure = 0;
                    // Retained objects from before the failure: record how they behave.
                    for element in retained.drain(..) {
                        tracer.op("stale-CurrentClassName", shell, || unsafe {
                            element.CurrentClassName()
                        });
                        tracer.op("stale-CurrentBoundingRectangle", shell, || unsafe {
                            element.CurrentBoundingRectangle()
                        });
                    }
                    // Fresh objects for the next attempt.
                    automation = None;
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        tracer.record(json!({"t": micros(), "event": "done"}));
        std::process::exit(0)
    }

    // ----------------------------------------------------------------- runner

    fn evidence(name: &str, value: Value) {
        println!("evidence {name}: {value}");
        if let Some(dir) = std::env::var_os("ELEVENCLOCK_EVIDENCE_DIR") {
            let _ = fs::create_dir_all(&dir);
            let _ = fs::write(
                Path::new(&dir).join(format!("uia-{name}.json")),
                serde_json::to_string_pretty(&value).unwrap(),
            );
        }
    }

    struct Provider {
        child: Child,
        hwnd: usize,
    }
    impl Provider {
        fn start(mode: &str, dir: &Path) -> Self {
            let file = dir.join(format!("hwnd-{}-{}", mode, micros()));
            let child = Command::new(exe())
                .args(["--provider", mode, file.to_str().unwrap()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            // Owned from here on: Drop reaps the child even if the wait below panics.
            let mut provider = Self { child, hwnd: 0 };
            let started = Instant::now();
            loop {
                if let Ok(text) = fs::read_to_string(&file)
                    && let Ok(hwnd) = text.trim().parse()
                {
                    provider.hwnd = hwnd;
                    return provider;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(15),
                    "provider never started"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        fn alive(&mut self) -> bool {
            matches!(self.child.try_wait(), Ok(None))
        }
        fn kill(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    impl Drop for Provider {
        fn drop(&mut self) {
            self.kill();
        }
    }

    fn inventory_under_supervision(
        hwnd: usize,
        deadline: Duration,
    ) -> (Result<Value, String>, Duration, Stats) {
        let spec = HelperSpec {
            program: exe(),
            args: vec!["--inventory".into(), hwnd.to_string().into()],
            env: Vec::new(),
            handshake: Vec::new(),
        };
        let stats = Stats::default();
        let started = Instant::now();
        let result = run_helper(&spec, deadline, &|| false, &stats);
        let elapsed = started.elapsed();
        let outcome = match result {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("unparsable: {e}")),
            Err(RunError::Failure(failure)) => Err(format!("{failure:?}")),
            Err(RunError::Cancelled) => Err("cancelled".into()),
        };
        (outcome, elapsed, stats)
    }

    fn read_log(path: &Path) -> Vec<Value> {
        fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }
    /// The trailing `begin` with no `end`, if the log currently ends mid-call.
    fn outstanding(log: &[Value]) -> Option<&Value> {
        let last = log.last()?;
        (last["event"] == "begin").then_some(last)
    }
    fn wait_for(what: &str, limit: Duration, mut condition: impl FnMut() -> bool) {
        let started = Instant::now();
        while !condition() {
            assert!(started.elapsed() < limit, "timed out: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn spawn_trace(spec: &str, log: &Path, seconds: u64, stale: Option<(&Path, &Path)>) -> Child {
        let mut command = Command::new(exe());
        command
            .args(["--trace", spec, log.to_str().unwrap(), &seconds.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some((ready, go)) = stale {
            command.args([ready.to_str().unwrap(), go.to_str().unwrap()]);
        }
        command.spawn().unwrap()
    }
    fn finish(child: &mut Child, limit: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < limit {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
        false
    }

    fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    // Case 1: the production inventory works against a healthy provider window.
    fn healthy_window_inventory() {
        let dir = scratch();
        let mut provider = Provider::start("normal", dir.path());
        let (result, elapsed, _) =
            inventory_under_supervision(provider.hwnd, Duration::from_secs(20));
        let summary = result.expect("inventory of a healthy window");
        evidence(
            "healthy-inventory",
            json!({"kind": "controlled provider (STATIC window)", "elapsed_ms": elapsed.as_millis() as u64, "summary": summary}),
        );
        assert!(summary["elements"].as_u64().unwrap() >= 1, "{summary}");
        provider.kill();
    }

    // Case 2: a genuinely hung provider. The production supervisor bounds the call,
    // reaps the helper, the parent stays responsive and a fresh request recovers.
    fn hung_provider_is_bounded_and_recovers() {
        let dir = scratch();
        let mut hung = Provider::start("hang", dir.path());
        let deadline = Duration::from_secs(4);
        let (result, elapsed, stats) = inventory_under_supervision(hung.hwnd, deadline);
        let outcome = match &result {
            Ok(summary) => json!({"completed": true, "summary": summary}),
            Err(why) => json!({"completed": false, "failure": why}),
        };
        let spawned = Stats::get(&stats.spawned);
        let exited = Stats::get(&stats.exited_observed);
        assert!(
            hung.alive(),
            "the provider itself is untouched by the client teardown"
        );
        evidence(
            "hung-provider",
            json!({
                "kind": "controlled provider: UI thread never pumps messages (real blocked UIA/COM calls)",
                "deadline_ms": deadline.as_millis() as u64, "elapsed_ms": elapsed.as_millis() as u64,
                "outcome": outcome, "helpers_spawned": spawned, "helper_exits_observed": exited,
            }),
        );
        assert_eq!(spawned, exited, "helper exit must be observed");
        assert!(
            elapsed < deadline + Duration::from_secs(3),
            "bounded: {elapsed:?}"
        );
        hung.kill();
        // Fresh provider, fresh request.
        let mut fresh = Provider::start("normal", dir.path());
        let (recovered, _, _) = inventory_under_supervision(fresh.hwnd, Duration::from_secs(20));
        assert!(recovered.is_ok(), "fresh request recovers: {recovered:?}");
        fresh.kill();
    }

    // Case 3: kill the provider while a UIA call is demonstrably outstanding.
    fn provider_death_during_an_outstanding_call() {
        let dir = scratch();
        let log = dir.path().join("trace.jsonl");
        let mut provider = Provider::start("hang", dir.path());
        let mut client = spawn_trace(&provider.hwnd.to_string(), &log, 25, None);
        // Wait until a call has been outstanding for at least half a second.
        let mut established = None;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(12) {
            let records = read_log(&log);
            if let Some(begin) = outstanding(&records) {
                let t = begin["t"].as_u64().unwrap();
                if micros().saturating_sub(t) >= 500_000 {
                    established = Some((
                        begin["op"].as_str().unwrap().to_string(),
                        begin["seq"].as_u64().unwrap(),
                        t,
                    ));
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let kill_issued = micros();
        provider.kill();
        let kill_done = micros();
        let client_exited = finish(&mut client, Duration::from_secs(30));
        let records = read_log(&log);
        let ended: Option<&Value> = established.as_ref().and_then(|(_, seq, _)| {
            records
                .iter()
                .find(|r| r["event"] == "end" && r["seq"] == *seq)
        });
        evidence(
            "provider-death-outstanding-call",
            json!({
                "kind": "controlled provider: process terminated while a client call was outstanding",
                "outstanding_established": established.is_some(),
                "outstanding_call": established.as_ref().map(|(op, seq, t)| json!({"op": op, "seq": seq, "begin_micros": t})),
                "provider_kill_issued_micros": kill_issued, "provider_kill_done_micros": kill_done,
                "call_end": ended,
                "client_returned_on_its_own": client_exited,
                "records": records.len(),
            }),
        );
        if let Some(end) = ended {
            assert!(
                end["t"].as_u64().unwrap() >= kill_issued,
                "the call was still outstanding at provider death: {end}"
            );
        }
        // The first failure observed is bounded and carries a real HRESULT (or the
        // call never returned and the supervisor-style kill above reaped the client).
        let failure = records
            .iter()
            .find(|r| r["event"] == "end" && r["ok"] == false);
        assert!(
            failure.is_some() || !client_exited || established.is_none(),
            "no failure or bounded exit recorded"
        );
        // Fresh request against a fresh provider recovers.
        let fresh = Provider::start("normal", dir.path());
        let log2 = dir.path().join("trace2.jsonl");
        let mut again = spawn_trace(&fresh.hwnd.to_string(), &log2, 20, None);
        assert!(finish(&mut again, Duration::from_secs(40)));
        let second = read_log(&log2);
        assert!(
            second.iter().any(|r| r["event"] == "done"),
            "fresh client finished"
        );
        assert!(
            second
                .iter()
                .all(|r| r["event"] != "end" || r["ok"] == true),
            "fresh objects work: {second:?}"
        );
    }

    // Case 4: elements retained across the provider's death fail; fresh ones work.
    fn retained_elements_fail_after_provider_death() {
        let dir = scratch();
        let (ready, go) = (dir.path().join("ready"), dir.path().join("go"));
        let log = dir.path().join("stale.jsonl");
        let mut provider = Provider::start("normal", dir.path());
        let mut client = spawn_trace(&provider.hwnd.to_string(), &log, 40, Some((&ready, &go)));
        wait_for(
            "client to acquire elements",
            Duration::from_secs(30),
            || ready.exists(),
        );
        provider.kill();
        fs::write(&go, "go").unwrap();
        assert!(
            finish(&mut client, Duration::from_secs(40)),
            "client returned after probing stale objects"
        );
        let records = read_log(&log);
        let stale: Vec<&Value> = records
            .iter()
            .filter(|r| {
                r["event"] == "end" && r["op"].as_str().is_some_and(|o| o.starts_with("stale-"))
            })
            .collect();
        evidence(
            "stale-retained-elements",
            json!({
                "kind": "controlled provider: elements retained across provider process death",
                "stale_calls": stale,
            }),
        );
        assert!(!stale.is_empty(), "stale probes ran");
        assert!(
            stale.iter().all(|r| r["ok"] == false),
            "retained objects must not return data after provider death: {stale:?}"
        );
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtSuspendProcess(process: windows::Win32::Foundation::HANDLE) -> i32;
        fn NtResumeProcess(process: windows::Win32::Foundation::HANDLE) -> i32;
    }

    /// Suspends a process so a client's call into it stays genuinely in flight, and
    /// guarantees it is resumed (or terminated) on every path so a failed test can
    /// never leave the desktop frozen.
    struct Freeze {
        process: windows::Win32::Foundation::HANDLE,
        suspended: bool,
    }
    impl Freeze {
        fn new(pid: u32) -> Self {
            use windows::Win32::System::Threading::{
                OpenProcess, PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE,
            };
            // SAFETY: opens the shell process with exactly the rights used below.
            let process =
                unsafe { OpenProcess(PROCESS_SUSPEND_RESUME | PROCESS_TERMINATE, false, pid) }
                    .expect("open the shell process");
            // SAFETY: valid handle with PROCESS_SUSPEND_RESUME.
            let status = unsafe { NtSuspendProcess(process) };
            assert!(status >= 0, "NtSuspendProcess failed: {status:#x}");
            Self {
                process,
                suspended: true,
            }
        }
        fn terminate(&mut self) {
            // SAFETY: valid handle with PROCESS_TERMINATE. A suspended process can be
            // terminated; the death is what the outstanding call must observe.
            let _ = unsafe { windows::Win32::System::Threading::TerminateProcess(self.process, 1) };
            self.suspended = false;
        }
    }
    impl Drop for Freeze {
        fn drop(&mut self) {
            // SAFETY: valid handle, closed exactly once.
            unsafe {
                if self.suspended {
                    let _ = NtResumeProcess(self.process);
                }
                let _ = windows::Win32::Foundation::CloseHandle(self.process);
            }
        }
    }

    // Case 5 (opt-in): real Explorer. To make "a call is outstanding when Explorer
    // dies" certain rather than lucky, Explorer is first *suspended* (a taskbar UIA
    // call then cannot complete), the call is observed in flight for at least half a
    // second, and only then is Explorer terminated. This is a deliberate freeze, not
    // a hang Explorer produced on its own, and is labelled that way in the evidence.
    fn real_explorer_restart_correlation() {
        if std::env::var_os("ELEVENCLOCK_UIA_EXPLORER").is_none() {
            println!("skipped: set ELEVENCLOCK_UIA_EXPLORER=1 (it restarts the user's shell)");
            return;
        }
        let dir = scratch();
        let mut attempts = Vec::new();
        let mut correlated = false;
        for attempt in 1..=4 {
            let log = dir.path().join(format!("explorer-{attempt}.jsonl"));
            let mut client = spawn_trace("taskbar", &log, 120, None);
            let steady_wait = Instant::now();
            let steady = loop {
                let ok_calls = read_log(&log)
                    .iter()
                    .filter(|r| r["event"] == "end" && r["ok"] == true)
                    .count();
                if ok_calls >= 30 {
                    break true;
                }
                if steady_wait.elapsed() > Duration::from_secs(60) {
                    break false;
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            if !steady {
                // The shell was not yet usable (still starting after the previous
                // restart). Record it rather than aborting the whole case.
                let _ = finish(&mut client, Duration::from_millis(1));
                attempts.push(json!({"attempt": attempt, "steady_state_timeout": true,
                    "log_tail": read_log(&log).iter().rev().take(6).collect::<Vec<_>>()}));
                evidence(
                    "explorer-restart-correlation",
                    json!({"correlated": correlated, "attempts": attempts}),
                );
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
            let before = probe::current_shell();
            let pid = before.expect("a running shell").0;
            let mut freeze = Freeze::new(pid);
            let freeze_t = micros();
            // Hold until a call has been in flight for >= 500 ms.
            let mut held = None;
            let waited = Instant::now();
            while waited.elapsed() < Duration::from_secs(10) {
                let records = read_log(&log);
                if let Some(begin) = outstanding(&records)
                    && micros().saturating_sub(begin["t"].as_u64().unwrap()) >= 500_000
                {
                    held = Some((
                        begin["op"].as_str().unwrap().to_string(),
                        begin["seq"].as_u64().unwrap(),
                        begin["t"].as_u64().unwrap(),
                    ));
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let kill_issued = micros();
            freeze.terminate();
            drop(freeze);
            let kill_done = micros();
            // Windows normally restarts the shell itself; start it if it does not.
            let restart = Instant::now();
            while target("taskbar").is_none() && restart.elapsed() < Duration::from_secs(25) {
                std::thread::sleep(Duration::from_millis(250));
            }
            if target("taskbar").is_none() {
                let _ = Command::new("explorer.exe").spawn();
                wait_for("taskbar to return", Duration::from_secs(40), || {
                    target("taskbar").is_some()
                });
            }
            let finished = finish(&mut client, Duration::from_secs(120));
            let records = read_log(&log);
            let held_end = held.as_ref().and_then(|(_, seq, _)| {
                records
                    .iter()
                    .find(|r| r["event"] == "end" && r["seq"] == *seq)
            });
            let held_failed = held_end
                .is_some_and(|e| e["ok"] == false && e["t"].as_u64().unwrap() >= kill_issued);
            let stale: Vec<&Value> = records
                .iter()
                .filter(|r| {
                    r["event"] == "end" && r["op"].as_str().is_some_and(|o| o.starts_with("stale-"))
                })
                .collect();
            let recovered = records.iter().find(|r| r["event"] == "recovered");
            let after = probe::current_shell();
            attempts.push(json!({
                "attempt": attempt,
                "kind": "real Explorer, suspended (NtSuspendProcess) to hold a taskbar UIA call in flight, then terminated",
                "shell_before": before.map(|s| json!({"pid": s.0, "started": s.1})),
                "shell_after": after.map(|s| json!({"pid": s.0, "started": s.1})),
                "freeze_micros": freeze_t, "terminate_issued_micros": kill_issued, "terminate_done_micros": kill_done,
                "call_held_outstanding": held.as_ref().map(|(op, seq, t)| json!({"op": op, "seq": seq, "begin_micros": t, "held_for_ms": (kill_issued.saturating_sub(*t)) / 1000})),
                "outstanding_call_result": held_end,
                "outstanding_call_failed_at_death": held_failed,
                "retained_object_calls": stale,
                "recovered_record": recovered,
                "client_finished": finished,
            }));
            let fresh_shell = matches!((before, after), (Some(b), Some(a)) if b != a);
            if held_failed && fresh_shell && recovered.is_some() {
                correlated = true;
            }
            evidence(
                "explorer-restart-correlation",
                json!({"correlated": correlated, "attempts": attempts}),
            );
            if correlated {
                break;
            }
            std::thread::sleep(Duration::from_secs(5));
        }
        evidence(
            "explorer-restart-correlation",
            json!({"correlated": correlated, "attempts": attempts}),
        );
        assert!(
            correlated,
            "no attempt held a UIA call outstanding until Explorer's death and then recovered; see evidence"
        );
    }

    pub fn main() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.first().map(String::as_str) {
            Some("--provider") => run_provider(&args[1..]),
            Some("--inventory") => run_inventory(&args[1..]),
            Some("--trace") => run_trace(&args[1..]),
            _ => {}
        }
        type TestFn = fn();
        let tests: Vec<(&str, TestFn)> = vec![
            ("healthy_window_inventory", healthy_window_inventory),
            (
                "hung_provider_is_bounded_and_recovers",
                hung_provider_is_bounded_and_recovers,
            ),
            (
                "provider_death_during_an_outstanding_call",
                provider_death_during_an_outstanding_call,
            ),
            (
                "retained_elements_fail_after_provider_death",
                retained_elements_fail_after_provider_death,
            ),
            (
                "real_explorer_restart_correlation",
                real_explorer_restart_correlation,
            ),
        ];
        let filters: Vec<_> = args.iter().filter(|a| !a.starts_with("--")).collect();
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
}
