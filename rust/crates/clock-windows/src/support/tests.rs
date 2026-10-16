//! Report format, redaction, status wording and refresh/clipboard rules. Pure
//! values: no OS, clock or clipboard is touched.
use super::*;
use crate::runtime::DiscoveryFailure;
use clock_core::{PhysicalRect, ShellGeneration, TaskbarId, TaskbarKey};

fn environment() -> Environment {
    Environment {
        app_version: "0.1.0".into(),
        commit: "482dd5f0a1b2".into(),
        windows: "10.0.26200.6899 (25H2)".into(),
        os_architecture: "x64".into(),
        app_architecture: "x64".into(),
    }
}
fn generated() -> DateTime<Utc> {
    DateTime::from_timestamp(1_792_000_000, 0).unwrap()
}
fn rect(l: i32, t: i32, w: i32, h: i32) -> PhysicalRect {
    PhysicalRect::new(l, t, w, h).unwrap()
}
fn bar(id: u64, state: OverlayState, reason: Option<Reason>) -> TaskbarDiagnostic {
    let primary = id == 1;
    TaskbarDiagnostic {
        key: TaskbarKey {
            generation: ShellGeneration(1),
            id: TaskbarId(id),
        },
        primary,
        state,
        reason: reason.clone(),
        monitor_rect: if primary {
            rect(0, 0, 1920, 1080)
        } else {
            rect(-2560, 0, 2560, 1440)
        },
        dpi: if primary { 144 } else { 96 },
        strategy: if primary {
            DiscoveryStrategy::UiAutomation
        } else {
            DiscoveryStrategy::NativeChildren
        },
        clock_located: reason != Some(Reason::ClockPositionUnverified),
    }
}
fn diagnostics(taskbars: Vec<TaskbarDiagnostic>) -> Diagnostics {
    Diagnostics {
        generation: ShellGeneration(1),
        paused: false,
        taskbars,
        last_discovery_failure: None,
        consecutive_failures: 0,
        discovery_disabled: false,
        shell_running: true,
        discovery_pending: false,
        discovery_completed: 1,
    }
}
fn visible(id: u64) -> TaskbarDiagnostic {
    bar(id, OverlayState::Visible, None)
}

// ---- redaction ----------------------------------------------------------------

fn redactor() -> Redactor {
    Redactor::new(Some("Jorim Tielemans"), Some(r"C:\Users\Jorim Tielemans"))
}

#[test]
fn redaction_removes_profile_folder_user_name_and_every_other_path() {
    let r = redactor();
    assert_eq!(
        r.apply(r"cannot open C:\Users\Jorim Tielemans\AppData\Local\ElevenClock-rs\x.dat: denied"),
        "cannot open <path>: denied"
    );
    assert_eq!(r.apply(r"see D:\data\file.txt."), "see <path>.");
    assert_eq!(r.apply("see D:/data/file.txt, then"), "see <path>, then");
    assert_eq!(r.apply(r"\\SERVER\share\doc.txt failed"), "<path> failed");
    assert_eq!(
        r.apply(r"\\?\C:\Windows\System32\x.dll"),
        "<path>",
        "extended-length prefix"
    );
    assert_eq!(
        r.apply("signed in as jorim tielemans (Jorim Tielemans)"),
        "signed in as <user> (<user>)"
    );
}

#[test]
fn redaction_leaves_ordinary_text_and_does_not_eat_inside_words() {
    let r = Redactor::new(Some("al"), None);
    assert_eq!(r.apply("value of al is final"), "value of <user> is final");
    assert_eq!(
        r.apply("Taskbar discovery did not finish in time."),
        "Taskbar discovery did not finish in time."
    );
    // A colon that is not a drive letter is not a path.
    assert_eq!(redactor().apply("step 1: next"), "step 1: next");
    assert_eq!(Redactor::default().apply(r"C:\x"), "<path>");
}

#[test]
fn reasons_are_flattened_capped_and_redacted() {
    let long = "x".repeat(500);
    let text = reason_text(&format!("line one\nline two {long}"));
    assert!(!text.contains('\n'));
    assert!(text.chars().count() <= REASON_LIMIT + 3, "{}", text.len());
    assert!(text.ends_with("..."));
    assert_eq!(
        reason_text(r"failed at C:\Users\someone\f.txt"),
        "failed at <path>"
    );
}

// ---- report -------------------------------------------------------------------

#[test]
fn report_format_is_stable() {
    let mut d = diagnostics(vec![
        visible(1),
        bar(
            2,
            OverlayState::Unavailable,
            Some(Reason::ClockPositionUnverified),
        ),
    ]);
    d.last_discovery_failure = None;
    let expected = "\
ElevenClock-rs diagnostics
Generated (UTC): 2026-10-14 17:46:40
Contains no user name, file paths or window titles. Nothing is sent anywhere; this text is only copied if you choose Copy diagnostics.

App version: 0.1.0
App commit: 482dd5f0a1b2
Windows: 10.0.26200.6899 (25H2)
Architecture: Windows x64, app x64

Clock replacement: Active on some taskbars
Summary: The ElevenClock-rs clock is showing on 1 of 2 taskbars. The Windows clock remains on the others. Taskbar 2: ElevenClock-rs could not verify where the Windows clock is on this taskbar, so it leaves the Windows clock alone. This Windows version may not be supported yet.
Paused by user: no
Windows Explorer running: yes
Taskbar discovery: last attempt succeeded

Taskbars: 2
1. Primary taskbar: clock replaced
   Monitor: 1920x1080 px at (0, 0), DPI 144 (150%)
   Discovery strategy: UI Automation; clock located: yes
2. Secondary taskbar: Windows clock in use: ElevenClock-rs could not verify where the Windows clock is on this taskbar, so it leaves the Windows clock alone. This Windows version may not be supported yet
   Monitor: 2560x1440 px at (-2560, 0), DPI 96 (100%)
   Discovery strategy: native child windows; clock located: no
";
    assert_eq!(report(&environment(), &d, generated()), expected);
}

#[test]
fn report_with_failures_and_no_taskbars() {
    let mut d = diagnostics(Vec::new());
    d.last_discovery_failure = Some(DiscoveryFailure::Deadline);
    d.consecutive_failures = 3;
    let text = report(&environment(), &d, generated());
    assert!(text.contains("Taskbars: 0\n"), "{text}");
    assert!(
        text.contains(
            "Taskbar discovery: last attempt failed: Taskbar discovery did not finish in time. (3 in a row)"
        ),
        "{text}"
    );
    assert!(
        text.contains("Clock replacement: Windows clock in use"),
        "{text}"
    );
}

#[test]
fn report_never_contains_paths_or_the_user_name_from_failure_reasons() {
    let mut d = diagnostics(vec![bar(
        1,
        OverlayState::Visible,
        Some(Reason::PlacementFailed(
            r"CreateWindowEx failed for C:\Users\realuser\AppData\x.exe".into(),
        )),
    )]);
    d.last_discovery_failure = Some(DiscoveryFailure::Rejected(
        r"bad data from \\HOST\share\helper.exe".into(),
    ));
    let text = report(&environment(), &d, generated());
    for forbidden in ["realuser", r"C:\", r"\\HOST", "AppData"] {
        assert!(!text.contains(forbidden), "{forbidden} leaked:\n{text}");
    }
    assert!(text.contains("<path>"), "{text}");
}

#[test]
fn report_has_no_window_title_field() {
    let text = report(&environment(), &diagnostics(vec![visible(1)]), generated());
    assert!(!text.to_ascii_lowercase().contains("title:"), "{text}");
}

#[test]
fn dpi_is_shown_as_percent_scale() {
    assert_eq!(dpi_text(96), "96 (100%)");
    assert_eq!(dpi_text(120), "120 (125%)");
    assert_eq!(dpi_text(144), "144 (150%)");
    assert_eq!(dpi_text(192), "192 (200%)");
    assert_eq!(dpi_text(0), "unknown");
}

// ---- status -------------------------------------------------------------------

#[test]
fn status_for_each_state() {
    let active = status(&diagnostics(vec![visible(1)]));
    assert_eq!(active.replacement, Replacement::Active);
    assert!(active.explanation.contains("your taskbar"));
    let both = status(&diagnostics(vec![visible(1), visible(2)]));
    assert!(both.explanation.contains("all 2 taskbars"));

    let mut paused = diagnostics(vec![bar(
        1,
        OverlayState::Hidden(HiddenReason::Paused),
        Some(Reason::NotShown),
    )]);
    paused.paused = true;
    let paused = status(&paused);
    assert_eq!(paused.replacement, Replacement::Paused);
    assert!(paused.explanation.contains("Resume"));

    let waiting = status(&diagnostics(vec![bar(
        1,
        OverlayState::Hidden(HiddenReason::Fullscreen),
        Some(Reason::NotShown),
    )]));
    assert_eq!(waiting.replacement, Replacement::Waiting);
    assert!(waiting.explanation.contains("full-screen"));

    let native = status(&diagnostics(vec![bar(
        1,
        OverlayState::Unavailable,
        Some(Reason::ClockPositionUnverified),
    )]));
    assert_eq!(native.replacement, Replacement::NativeClock);
    assert!(native.explanation.contains("Windows clock is showing"));
    assert!(native.explanation.contains("not be supported"));
}

#[test]
fn status_names_the_fix_for_a_clock_that_does_not_fit() {
    let overflow = crate::render::Overflow {
        required: crate::render::PixelSize {
            width: 300,
            height: 40,
        },
        available: crate::render::PixelSize {
            width: 200,
            height: 40,
        },
    };
    let s = status(&diagnostics(vec![bar(
        1,
        OverlayState::Visible,
        Some(Reason::Overflow(overflow)),
    )]));
    assert_eq!(s.replacement, Replacement::NativeClock);
    assert!(
        s.explanation.contains("shorter format"),
        "{}",
        s.explanation
    );
}

#[test]
fn status_without_taskbars() {
    let mut d = diagnostics(Vec::new());
    assert_eq!(status(&d).replacement, Replacement::Searching);
    d.shell_running = false;
    let s = status(&d);
    assert_eq!(s.replacement, Replacement::NativeClock);
    assert!(s.explanation.contains("Explorer is not running"));
    d.shell_running = true;
    d.last_discovery_failure = Some(DiscoveryFailure::NoTaskbar);
    let s = status(&d);
    assert_eq!(s.replacement, Replacement::NativeClock);
    assert!(s.explanation.contains("tries again automatically"));
    d.discovery_disabled = true;
    assert!(status(&d).explanation.contains("Exit and start"));
}

#[test]
fn status_text_leads_with_version_and_windows_build() {
    let text = status_text(&environment(), &diagnostics(vec![visible(1)]));
    assert!(text.starts_with("ElevenClock-rs version 0.1.0\nWindows 10.0.26200.6899 (25H2)\n"));
    assert!(text.contains("Status: Active\n"));
}

// ---- refresh and clipboard -------------------------------------------------------

#[test]
fn refresh_waits_for_a_new_answer_and_never_forever() {
    let mut d = diagnostics(vec![visible(1)]);
    let watch = RefreshWatch::start(&d);
    assert_eq!(watch.progress(&d, Duration::ZERO), Progress::Waiting);
    d.discovery_pending = true;
    assert_eq!(
        watch.progress(&d, Duration::from_secs(1)),
        Progress::Waiting
    );
    // The answer is in, but another discovery was already queued behind it.
    d.discovery_completed = 2;
    assert_eq!(
        watch.progress(&d, Duration::from_secs(2)),
        Progress::Waiting
    );
    d.discovery_pending = false;
    assert_eq!(watch.progress(&d, Duration::from_secs(3)), Progress::Done);
    // A helper that never answers: stop waiting at the limit.
    let stuck = diagnostics(vec![visible(1)]);
    assert_eq!(
        watch.progress(&stuck, REFRESH_LIMIT - Duration::from_millis(1)),
        Progress::Waiting
    );
    assert_eq!(watch.progress(&stuck, REFRESH_LIMIT), Progress::TimedOut);
}

#[test]
fn refresh_is_done_at_once_when_discovery_is_off() {
    let mut d = diagnostics(Vec::new());
    let watch = RefreshWatch::start(&d);
    d.discovery_disabled = true;
    assert_eq!(watch.progress(&d, Duration::ZERO), Progress::Done);
}

#[test]
fn a_failed_refresh_still_finishes() {
    let mut d = diagnostics(vec![visible(1)]);
    let watch = RefreshWatch::start(&d);
    d.discovery_completed += 1;
    d.last_discovery_failure = Some(DiscoveryFailure::Deadline);
    assert_eq!(watch.progress(&d, Duration::from_secs(5)), Progress::Done);
}

#[test]
fn copy_messages_say_whether_the_clipboard_changed() {
    assert_eq!(copy_message(Ok(())), "Diagnostics copied to the clipboard.");
    let busy = copy_message(Err(ClipboardError {
        fault: ClipboardFault::Busy,
        replaced: false,
    }));
    assert!(busy.contains("Another program is using the clipboard"));
    assert!(busy.ends_with("Your clipboard was not changed."));
    let late = copy_message(Err(ClipboardError {
        fault: ClipboardFault::Rejected,
        replaced: true,
    }));
    assert!(late.ends_with("The clipboard was cleared."));
}

#[test]
fn a_display_left_out_in_settings_is_a_choice_not_a_partial_failure() {
    let d = diagnostics(vec![
        visible(1),
        bar(
            2,
            OverlayState::Hidden(HiddenReason::Paused),
            Some(Reason::NotShown),
        ),
    ]);
    let s = status(&d);
    assert_eq!(s.replacement, Replacement::Active);
    assert!(
        s.explanation
            .starts_with("The ElevenClock-rs clock is showing on your taskbar.")
    );
    assert!(
        s.explanation.contains("primary display only"),
        "{}",
        s.explanation
    );
    assert!(
        report(&environment(), &d, generated())
            .contains("2. Secondary taskbar: not used: this display is not selected in Settings")
    );
}
