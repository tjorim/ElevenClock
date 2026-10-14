//! Private development harness. Never included in the installed clock-app.
#![cfg_attr(windows, windows_subsystem = "windows")]
#[cfg(any(windows, test))]
#[path = "../../clock-windows/examples/taskbar_probe/isolation.rs"]
mod isolation;

#[cfg(not(windows))]
fn main() {
    eprintln!("overlay_spike requires Windows");
    std::process::exit(1);
}
#[cfg(windows)]
fn main() {
    if let Err(error) = experiment::run() {
        // GUI-subsystem harness errors go to the explicitly supplied evidence
        // file, not an end-user console or product diagnostic interface.
        if let Some(path) = std::env::args().nth(1).filter(|p| p != "--helper") {
            let _ = std::fs::write(
                path,
                serde_json::json!({"status":"failed","reason":error.to_string()}).to_string(),
            );
        }
        std::process::exit(1);
    }
}
#[cfg(windows)]
mod experiment {
    use super::isolation::{Completion, HelperJob, supervise};
    use clock_core::PhysicalRect;
    use clock_windows::{
        overlay::{self, ExperimentSlot, Update},
        prototype_interaction,
    };
    use serde_json::{Value, json};
    use std::{
        fs::{self, OpenOptions},
        io::{Read, Write},
        process::{Command, Stdio},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };
    use windows::Win32::UI::HiDpi::{
        AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        GetThreadDpiAwarenessContext,
    };
    type Error = Box<dyn std::error::Error>;
    fn rect(r: PhysicalRect) -> Value {
        json!([r.left(), r.top(), r.width(), r.height()])
    }
    fn encode(s: &ExperimentSlot) -> Value {
        json!({"clock":rect(s.clock),"taskbar":rect(s.taskbar),"monitor":rect(s.monitor),"dpi":s.dpi,"shell_pid":s.shell_pid,"shell_started":s.shell_started,"primary":s.primary})
    }
    fn decode(v: &Value) -> Result<ExperimentSlot, Error> {
        let r = |name: &str| -> Result<PhysicalRect, Error> {
            let a = v[name].as_array().ok_or("Missing rectangle")?;
            if a.len() != 4 {
                return Err("Invalid rectangle".into());
            }
            let n = |i: usize| -> Result<i32, Error> {
                Ok(i32::try_from(a[i].as_i64().ok_or("Invalid edge")?)?)
            };
            Ok(PhysicalRect::new(n(0)?, n(1)?, n(2)?, n(3)?)?)
        };
        Ok(ExperimentSlot {
            clock: r("clock")?,
            taskbar: r("taskbar")?,
            monitor: r("monitor")?,
            dpi: u32::try_from(v["dpi"].as_u64().ok_or("Missing DPI")?)?,
            shell_pid: u32::try_from(v["shell_pid"].as_u64().ok_or("Missing shell PID")?)?,
            shell_started: v["shell_started"].as_u64().ok_or("Missing shell epoch")?,
            primary: v["primary"].as_bool().ok_or("Missing primary flag")?,
        })
    }
    struct Output(std::path::PathBuf);
    impl Drop for Output {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    fn helper(request: &Value, sequence: u64) -> Result<Value, Error> {
        let path = std::env::temp_dir().join(format!("r03-{}-{sequence}.json", std::process::id()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let _output = Output(path.clone());
        let mut child = Command::new(std::env::current_exe()?)
            .arg("--helper")
            .arg(request.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::from(file))
            .stderr(Stdio::null())
            .spawn()?;
        let _job = match HelperJob::attach(&child) {
            Ok(job) => job,
            Err(e) => {
                let _ = supervise(&mut child, Duration::ZERO);
                return Err(e.into());
            }
        };
        child
            .stdin
            .take()
            .ok_or("Missing handshake")?
            .write_all(b"r03-go")?;
        match supervise(&mut child, Duration::from_secs(5))? {
            Completion::Exited(status) if status.success() => {
                if fs::metadata(&path)?.len() > 1024 * 1024 {
                    return Err("Oversized result".into());
                }
                let value: Value = serde_json::from_slice(&fs::read(&path)?)?;
                if value["sequence"].as_u64() != Some(sequence) {
                    return Err("Stale helper result".into());
                }
                Ok(value)
            }
            Completion::Deadline { teardown_micros } => Err(format!(
                "Helper deadline; teardown {teardown_micros} us; native access restored"
            )
            .into()),
            Completion::Exited(status) => {
                Err(format!("Helper failed: {status}; native access restored").into())
            }
        }
    }
    pub fn run() -> Result<(), Error> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if unsafe {
            !AreDpiAwarenessContextsEqual(
                GetThreadDpiAwarenessContext(),
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            )
            .as_bool()
        } {
            return Err("Embedded DPI-v2 manifest required".into());
        }
        if args.first().map(String::as_str) == Some("--helper") {
            let request: Value = serde_json::from_str(args.get(1).ok_or("Missing request")?)?;
            let sequence = request["sequence"].as_u64().ok_or("Missing sequence")?;
            let mut handshake = [0; 6];
            std::io::stdin().read_exact(&mut handshake)?;
            if &handshake != b"r03-go" {
                return Err("Invalid handshake".into());
            }
            let action = (|| -> Result<Value, Error> {
                if request["action"] == "invoke" {
                    let slot = decode(&request["slot"])?;
                    Ok(json!({"action": prototype_interaction::invoke(&slot, request["fallback"].as_bool().unwrap_or(false))?}))
                } else {
                    if std::env::var_os("ELEVENCLOCK_R03_REJECT_DISCOVERY").is_some() {
                        return Err("Injected unsupported-discovery result; no geometry accepted".into());
                    }
                    Ok(json!({"slots": prototype_interaction::discover(sequence, request["all"].as_bool().unwrap_or(false))?.iter().map(encode).collect::<Vec<_>>()}))
                }
            })().unwrap_or_else(|error| json!({"failed":error.to_string()}));
            serde_json::to_writer(
                std::io::stdout().lock(),
                &json!({"sequence":sequence,"result":action}),
            )?;
            return Ok(());
        }
        if args.len() != 3
            || !matches!(
                args[2].as_str(),
                "primary" | "all" | "primary-win-n" | "all-win-n"
            )
        {
            return Err("Development usage: overlay_spike evidence.json seconds primary|all|primary-win-n|all-win-n".into());
        }
        let duration: u64 = args[1].parse()?;
        if !(1..=300).contains(&duration) {
            return Err("Duration must be 1..300 seconds".into());
        }
        let path = std::path::PathBuf::from(&args[0]);
        let all = args[2].starts_with("all");
        let fallback = args[2].ends_with("win-n");
        let (tx, rx) = mpsc::sync_channel(1);
        let (click_tx, click_rx) = mpsc::sync_channel::<ExperimentSlot>(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = std::thread::spawn(move || {
            let mut reports = Vec::new();
            let mut sequence = 0;
            while !worker_stop.load(Ordering::Relaxed) {
                sequence += 1;
                let click = click_rx.try_recv().ok();
                let request = match &click {
                    Some(slot) => {
                        json!({"sequence":sequence,"action":"invoke","slot":encode(slot),"fallback":fallback})
                    }
                    None => json!({"sequence":sequence,"action":"discover","all":all}),
                };
                // Coverage is removed before every blocking discovery; stale
                // geometry cannot persist while a provider stalls.
                if tx.send(Update::Failed).is_err() {
                    break;
                }
                let result = helper(&request, sequence);
                match result {
                    Ok(report) => {
                        if click.is_none() {
                            let slots = report["result"]["slots"]
                                .as_array()
                                .ok_or("Missing slots")
                                .and_then(|a| {
                                    a.iter()
                                        .map(decode)
                                        .collect::<Result<Vec<_>, _>>()
                                        .map_err(|_| "Invalid slot")
                                });
                            if let Ok(slots) = slots
                                && tx.send(Update::Slots(slots)).is_err()
                            {
                                break;
                            }
                        }
                        reports.push(report);
                        if click.is_some() && tx.send(Update::InteractionComplete).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        reports.push(json!({"sequence":sequence,"fatal":error.to_string()}));
                        let _ = fs::write(
                            &path,
                            json!({"status":"fatal_supervision_failure","reports":reports})
                                .to_string(),
                        );
                        break;
                    }
                }
                let (build, ubr) = clock_windows::taskbar::probe::os_build().unwrap_or_default();
                let evidence = json!({"status":"experimental_not_production_go","build":build,"ubr":ubr,"architecture":std::env::consts::ARCH,"reports":reports});
                if fs::write(&path, evidence.to_string()).is_err() {
                    let _ = tx.send(Update::Failed);
                    break;
                }
                // Bounded click wait; coalesced channel prevents repeated input.
                for _ in 0..20 {
                    if worker_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        });
        let result = overlay::run(rx, click_tx, Duration::from_secs(duration));
        stop.store(true, Ordering::Relaxed);
        let _ = worker.join();
        result?;
        Ok(())
    }
}
