//! Development artifact only; not linked into or shipped with clock-app.
#[cfg(any(windows, test))]
mod isolation;

#[cfg(any(windows, test))]
fn current_envelope(report: &serde_json::Value, requested: u64, current: bool) -> bool {
    current
        && report["schema"].as_u64() == Some(1)
        && report["generation"].as_u64() == Some(requested)
}

#[cfg(test)]
mod tests {
    use super::current_envelope;
    use serde_json::json;
    #[test]
    fn rejects_stale_missing_or_malformed_generation_envelopes() {
        for value in [
            json!({"schema":1,"generation":6}),
            json!({"schema":1}),
            json!({"schema":1,"generation":"7"}),
            json!({"schema":2,"generation":7}),
        ] {
            assert!(!current_envelope(&value, 7, true));
        }
        let value = json!({"schema":1,"generation":7});
        assert!(!current_envelope(&value, 7, false));
        assert!(current_envelope(&value, 7, true));
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The taskbar probe requires an interactive Windows desktop.");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_probe::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod windows_probe {
    use super::isolation::{Completion, supervise};
    use clock_core::{PhysicalRect, ShellGeneration};
    use clock_windows::taskbar::{ProbeQueue, probe};
    use serde_json::{Value, json};
    use std::{
        fs::{self, OpenOptions},
        io::{self, Read, Write},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    struct TemporaryOutput(std::path::PathBuf);
    impl Drop for TemporaryOutput {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };

    fn rect(value: Option<PhysicalRect>) -> Value {
        value
            .map(
                |r| json!({"left":r.left(), "top":r.top(), "right":r.right(), "bottom":r.bottom()}),
            )
            .unwrap_or(Value::Null)
    }

    fn helper(generation: u64) -> Result<Value, Box<dyn std::error::Error>> {
        // SAFETY: first native operation in a dedicated development executable.
        // If awareness cannot be established, do not report virtualized pixels.
        unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        }
        let (build, revision) = probe::os_build()?;
        let records = probe::collect(ShellGeneration(generation))?;
        let taskbars: Vec<_> = records.into_iter().map(|record| {
            let s = record.snapshot;
            json!({
                "id":s.key.id.0, "generation":s.key.generation.0,
                "shell_pid":record.shell_pid, "shell_started_filetime":record.shell_started,
                "taskbar_rect":rect(Some(s.taskbar_rect)), "clock_rect":Value::Null,
                "monitor_id":s.monitor.0, "monitor_rect":rect(Some(s.monitor_rect)),
                "dpi":s.dpi, "primary":s.primary, "visibility":format!("{:?}",s.visibility),
                "elapsed_micros":record.elapsed_micros as u64,
                "native_children":record.native_children.iter().map(|c| json!({"class":c.class,"rect":rect(c.rect),"visible":c.visible})).collect::<Vec<_>>(),
                "native_inventory_incomplete":record.native_inventory_incomplete,
                "uia":{
                    "elapsed_micros":record.uia.elapsed_micros as u64,
                    "longest_call_micros":record.uia.longest_call_micros as u64,
                    "calls":record.uia.calls.iter().map(|c| json!({"operation":c.operation,"elapsed_micros":c.elapsed_micros as u64,"succeeded":c.succeeded})).collect::<Vec<_>>(),
                    "failures":record.uia.failures, "truncated":record.uia.truncated,
                    "elements":record.uia.elements.iter().map(|e| json!({
                        "parent":e.parent, "automation_id":e.automation_id, "class":e.class,
                        "framework":e.framework, "control_type":e.control_type,
                        "process_id":e.process_id, "offscreen":e.offscreen,
                        "rect":rect(e.rect), "elapsed_micros":e.elapsed_micros as u64
                    })).collect::<Vec<_>>()
                }
            })
        }).collect();
        Ok(
            json!({"schema":1,"generation":generation,"build":build,"ubr":revision,
            "architecture":std::env::consts::ARCH,"status":"unsupported_no_verified_selector", "taskbars":taskbars}),
        )
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args.first().map(String::as_str) == Some("--helper") {
            if args.len() != 2 {
                return Err("Invalid private helper invocation".into());
            }
            let generation = args[1].parse()?;
            // Do no native work until assignment to the parent's kill-on-close
            // job succeeds. Parent loss before assignment closes stdin (EOF).
            let mut handshake = [0u8; 6];
            io::stdin().read_exact(&mut handshake)?;
            if &handshake != b"r02-go" {
                return Err("Invalid helper handshake".into());
            }
            // Developer-only fault injection: prove Windows process teardown
            // with a confirmed blocked helper. This is not a simulated UIA result.
            if std::env::var_os("ELEVENCLOCK_RS_R02_SIMULATE_STALL").is_some() {
                if let Some(path) = std::env::var_os("ELEVENCLOCK_RS_R02_STALL_READY") {
                    fs::write(path, std::process::id().to_string())?;
                }
                std::thread::sleep(Duration::from_secs(60));
                return Err("Stall fixture unexpectedly reached its end".into());
            }
            let report = match helper(generation) {
                Ok(report) => report,
                Err(error) => {
                    json!({"schema":1,"generation":generation,"status":"probe_error","reason":error.to_string()})
                }
            };
            serde_json::to_writer(io::stdout().lock(), &report)?;
            return Ok(());
        }
        // Optional sample count is a developer harness parameter, not an app CLI.
        let samples: u64 = match args.as_slice() {
            [] => 1,
            [count] => count.parse()?,
            _ => {
                return Err("Usage (development only): taskbar_probe [sample-count: 1..60]".into());
            }
        };
        if !(1..=60).contains(&samples) {
            return Err("Sample count must be 1..60".into());
        }
        let mut queue = ProbeQueue::default();
        for generation in 1..=samples {
            queue.request(ShellGeneration(generation));
            let request = queue.start().ok_or("Another helper is still active")?;
            let path = std::env::temp_dir().join(format!(
                "elevenclock-rs-r02-{}-{generation}.json",
                std::process::id()
            ));
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            let _temporary_output = TemporaryOutput(path.clone());
            let started = Instant::now();
            let mut child = Command::new(std::env::current_exe()?)
                .args(["--helper", &request.0.to_string()])
                .stdin(Stdio::piped())
                .stdout(Stdio::from(file))
                .stderr(Stdio::null())
                .spawn()?;
            let _job = match super::isolation::HelperJob::attach(&child) {
                Ok(job) => job,
                Err(error) => {
                    let _ = supervise(&mut child, Duration::ZERO);
                    return Err(format!("Helper isolation unavailable; no retries: {error}").into());
                }
            };
            child
                .stdin
                .take()
                .ok_or("Missing helper handshake pipe")?
                .write_all(b"r02-go")?;
            // Fatal errors stop this process before any replacement is spawned.
            let completion = supervise(&mut child, Duration::from_secs(5))?;
            let current = queue.finish(request);
            let report = match completion {
                Completion::Deadline { teardown_micros } => {
                    json!({"schema":1,"generation":request.0,
                    "status":"deadline","teardown_micros":teardown_micros as u64,"clock_rect":Value::Null})
                }
                Completion::Exited(status) if !status.success() => {
                    json!({"schema":1,"generation":request.0,"status":"helper_exit","exit":status.code()})
                }
                Completion::Exited(_) => {
                    if fs::metadata(&path)?.len() > 8 * 1024 * 1024 {
                        return Err("Helper output exceeds 8 MiB; no result accepted".into());
                    }
                    let report: Value = serde_json::from_slice(&fs::read(&path)?)?;
                    if !super::current_envelope(&report, request.0, current) {
                        json!({"schema":1,"generation":request.0,"status":"stale_generation","clock_rect":Value::Null})
                    } else {
                        report
                    }
                }
            };
            fs::remove_file(path)?;
            let report = json!({"supervisor_elapsed_micros":started.elapsed().as_micros() as u64,"result":report});
            let mut output = io::stdout().lock();
            serde_json::to_writer(&mut output, &report)?;
            writeln!(output)?;
            drop(output);
            if generation < samples {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        Ok(())
    }
}
