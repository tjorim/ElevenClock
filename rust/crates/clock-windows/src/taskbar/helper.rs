//! The private discovery helper role of the app executable (Windows only).
//!
//! The parent launches `mainspring.exe` with one fixed private marker and a
//! generation (see `instance::Role`), assigns the process to a kill-on-close
//! job, and only then writes the handshake this function waits for. Without that
//! handshake the process does nothing and exits: launching the marker by hand
//! (it is not a user command) cannot make the helper touch Explorer.
//!
//! All blocking shell and UI Automation work happens here, on this process' main
//! thread, so an unresponsive provider can only ever block this expendable
//! process, which the parent terminates at its deadline.
use super::{
    candidate::{self, CandidateElement},
    probe,
    protocol::{self, ClockRecord, Report, ShellRecord, TaskbarRecord},
};
use clock_core::ShellGeneration;
use std::io::{Read, Write};
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    GetThreadDpiAwarenessContext,
};

const HANDSHAKE_NATIVE: &[u8] = b"r08-go:native";
const HANDSHAKE_UIA: &[u8] = b"r08-go:uia";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Native,
    Uia,
}

/// Read the parent's handshake: one short line. EOF (the parent died before
/// assigning the job) or anything unexpected yields `None` and no native work.
fn read_handshake(mut input: impl Read) -> Option<Mode> {
    let mut line = Vec::with_capacity(16);
    let mut byte = [0u8; 1];
    while line.len() <= 32 {
        match input.read(&mut byte) {
            Ok(1) if byte[0] == b'\n' => break,
            Ok(1) => line.push(byte[0]),
            _ => return None,
        }
    }
    match line.as_slice() {
        HANDSHAKE_NATIVE => Some(Mode::Native),
        HANDSHAKE_UIA => Some(Mode::Uia),
        _ => None,
    }
}

fn report(generation: u64, mode: Mode) -> windows::core::Result<Report> {
    let observations = probe::collect_with(ShellGeneration(generation), mode == Mode::Uia)?;
    let shell = observations
        .first()
        .map(|o| (o.shell_pid, o.shell_started))
        .or_else(probe::current_shell)
        .map(|(pid, started)| ShellRecord { pid, started });
    let taskbars = observations
        .iter()
        .map(|o| {
            let s = &o.snapshot;
            let clock = (mode == Mode::Uia)
                .then(|| {
                    let elements: Vec<_> = o
                        .uia
                        .elements
                        .iter()
                        .map(|e| CandidateElement {
                            parent: e.parent,
                            automation_id: e.automation_id.clone(),
                            class: e.class.clone(),
                            control_type: e.control_type,
                            process_id: e.process_id,
                            offscreen: e.offscreen,
                            rect: e.rect,
                        })
                        .collect();
                    let complete = !o.uia.truncated
                        && o.uia.failures.is_empty()
                        && !o.native_inventory_incomplete;
                    candidate::select(&elements, o.shell_pid, complete)
                })
                .flatten()
                .map(|c| ClockRecord {
                    rect: protocol::rect_to_wire(c.rect),
                    adjacent: c.adjacent.into_iter().map(protocol::rect_to_wire).collect(),
                    exclusions_complete: c.exclusions_complete,
                });
            TaskbarRecord {
                id: s.key.id.0,
                hwnd: o.hwnd as u64,
                taskbar: protocol::rect_to_wire(s.taskbar_rect),
                monitor_id: s.monitor.0,
                monitor: protocol::rect_to_wire(s.monitor_rect),
                dpi: s.dpi,
                primary: s.primary,
                visible: s.visibility == clock_core::TaskbarVisibility::Visible,
                clock,
            }
        })
        .collect();
    Ok(Report {
        schema: protocol::SCHEMA,
        generation,
        shell,
        taskbars,
    })
}

/// Entry point; returns the process exit code. Output is the JSON report on
/// stdout (a file the parent created); nothing else is ever printed.
pub fn run(generation: u64) -> i32 {
    let Some(mode) = read_handshake(std::io::stdin().lock()) else {
        return 2;
    };
    // The embedded manifest makes the app per-monitor-v2 aware; without it every
    // rectangle would be virtualized and wrong, so refuse rather than guess.
    // SAFETY: pure query of the calling thread's DPI awareness.
    let aware = unsafe {
        AreDpiAwarenessContextsEqual(
            GetThreadDpiAwarenessContext(),
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        )
        .as_bool()
    };
    if !aware {
        return 3;
    }
    match report(generation, mode) {
        Ok(report) => {
            let mut stdout = std::io::stdout().lock();
            if serde_json::to_writer(&mut stdout, &report).is_ok() && stdout.flush().is_ok() {
                0
            } else {
                5
            }
        }
        Err(_) => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_exact_handshakes_start_work() {
        assert_eq!(read_handshake(&b"r08-go:native\n"[..]), Some(Mode::Native));
        assert_eq!(read_handshake(&b"r08-go:uia\n"[..]), Some(Mode::Uia));
        // EOF (parent lost before job assignment), wrong text, overlong or no newline.
        for bad in [
            &b""[..],
            b"r08-go:native",
            b"r02-go\n",
            b"r08-go:uia extra\n",
            b"\n",
            &[b'x'; 64],
        ] {
            assert_eq!(read_handshake(bad), None, "{bad:?}");
        }
    }
}
