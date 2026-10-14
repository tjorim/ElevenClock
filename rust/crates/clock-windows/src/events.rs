//! Platform notifications reduced to a small value vocabulary (R08).
//!
//! Window procedures and WinEvent callbacks only classify a notification and
//! enqueue a [`Signal`]; they never touch runtime state, discovery or rendering.
//! [`Pending`] coalesces bursts, so a display change that arrives as several
//! messages becomes one topology refresh. Everything in this module is portable
//! and unit tested; only the registration code in `native` needs Windows.
use std::time::Duration;

/// Documented fallback cadence for shell state that events did not report.
pub const RECOVERY_POLL: Duration = Duration::from_secs(2);
/// Short, bounded polling while a taskbar is animating (auto-hide slide).
pub const TRANSITION_POLL: Duration = Duration::from_millis(100);
/// Upper bound on consecutive transition polls before returning to `RECOVERY_POLL`.
pub const TRANSITION_POLL_LIMIT: u8 = 15;

/// Raw Win32 values, duplicated so mapping stays testable off Windows. A
/// Windows-only test asserts they equal the `windows` crate's definitions.
pub mod raw {
    pub const WM_SYSCOLORCHANGE: u32 = 0x0015;
    pub const WM_SETTINGCHANGE: u32 = 0x001A;
    pub const WM_TIMECHANGE: u32 = 0x001E;
    pub const WM_ENDSESSION: u32 = 0x0016;
    pub const WM_DISPLAYCHANGE: u32 = 0x007E;
    pub const WM_POWERBROADCAST: u32 = 0x0218;
    pub const WM_DPICHANGED: u32 = 0x02E0;
    pub const WM_THEMECHANGED: u32 = 0x031A;
    pub const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
    pub const PBT_APMSUSPEND: usize = 0x0004;
    pub const PBT_APMRESUMESUSPEND: usize = 0x0007;
    pub const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
    pub const WTS_CONSOLE_CONNECT: usize = 1;
    pub const WTS_CONSOLE_DISCONNECT: usize = 2;
    pub const WTS_REMOTE_CONNECT: usize = 3;
    pub const WTS_REMOTE_DISCONNECT: usize = 4;
    pub const WTS_SESSION_LOCK: usize = 7;
    pub const WTS_SESSION_UNLOCK: usize = 8;
    pub const SPI_SETWORKAREA: usize = 0x002F;
}

/// What a WinEvent hook reported, already filtered to shell-relevant objects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellEvent {
    /// A taskbar window moved, resized, showed or hid (auto-hide slide, resize).
    TaskbarChanged,
    /// Foreground or minimize state changed: fullscreen coverage may differ.
    ForegroundChanged,
}

/// Inputs to the runtime. Variants carry no handles or borrowed data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signal {
    /// Display configuration, DPI or taskbar settings: refresh topology.
    TopologyChanged,
    /// Colors, theme or high contrast may have changed.
    AppearanceMaybeChanged,
    /// System time, zone or DST rules changed: take a fresh local snapshot.
    TimeChanged,
    Suspended,
    /// Resume from sleep/hibernate: time, topology and Explorer all need rechecks.
    Resumed,
    SessionLocked,
    SessionUnlocked,
    /// The registered `TaskbarCreated` broadcast: Explorer replaced its taskbar.
    TaskbarCreated,
    Shell(ShellEvent),
    /// The session is ending (log off or shutdown).
    EndSession,
}

/// Maps a window message to a [`Signal`]. `setting` is the `WM_SETTINGCHANGE`
/// string payload when present. `taskbar_created` is the registered message id.
pub fn signal_for_message(
    message: u32,
    wparam: usize,
    setting: Option<&str>,
    taskbar_created: u32,
) -> Option<Signal> {
    use raw::*;
    match message {
        m if taskbar_created != 0 && m == taskbar_created => Some(Signal::TaskbarCreated),
        WM_DISPLAYCHANGE | WM_DPICHANGED => Some(Signal::TopologyChanged),
        WM_TIMECHANGE => Some(Signal::TimeChanged),
        WM_THEMECHANGED | WM_SYSCOLORCHANGE => Some(Signal::AppearanceMaybeChanged),
        WM_SETTINGCHANGE => match setting {
            // Windows posts this when taskbar auto-hide, size or alignment change.
            Some("TraySettings") => Some(Signal::TopologyChanged),
            Some("ImmersiveColorSet" | "WindowsThemeElement") => {
                Some(Signal::AppearanceMaybeChanged)
            }
            Some(_) if wparam == SPI_SETWORKAREA => Some(Signal::TopologyChanged),
            None if wparam == SPI_SETWORKAREA => Some(Signal::TopologyChanged),
            // Other settings (locale, fonts, accessibility) can change appearance
            // or time formatting inputs; appearance is deduplicated downstream.
            _ => Some(Signal::AppearanceMaybeChanged),
        },
        WM_POWERBROADCAST => match wparam {
            PBT_APMSUSPEND => Some(Signal::Suspended),
            PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => Some(Signal::Resumed),
            _ => None,
        },
        WM_WTSSESSION_CHANGE => match wparam {
            WTS_SESSION_LOCK => Some(Signal::SessionLocked),
            WTS_SESSION_UNLOCK => Some(Signal::SessionUnlocked),
            // Fast user switching and remote sessions change the display set.
            WTS_CONSOLE_CONNECT | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_CONNECT
            | WTS_REMOTE_DISCONNECT => Some(Signal::TopologyChanged),
            _ => None,
        },
        WM_ENDSESSION if wparam != 0 => Some(Signal::EndSession),
        _ => None,
    }
}

/// Relevance filter for WinEvent notifications. Called inside the hook callback,
/// so it takes only values the callback already has and does no allocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WinEventKind {
    Foreground,
    MinimizeEnd,
    MoveSizeEnd,
    ObjectShow,
    ObjectHide,
    ObjectLocationChange,
}
pub fn shell_event_for(
    kind: WinEventKind,
    is_window_object: bool,
    is_taskbar_window: impl FnOnce() -> bool,
) -> Option<ShellEvent> {
    match kind {
        // Low-rate system events: any window may have become fullscreen.
        WinEventKind::Foreground | WinEventKind::MinimizeEnd | WinEventKind::MoveSizeEnd => {
            Some(ShellEvent::ForegroundChanged)
        }
        // High-rate object events: only taskbar windows are relevant, and only
        // whole-window notifications (not carets, cursors or children).
        WinEventKind::ObjectShow | WinEventKind::ObjectHide | WinEventKind::ObjectLocationChange => {
            (is_window_object && is_taskbar_window()).then_some(ShellEvent::TaskbarChanged)
        }
    }
}

const ORDERED_CAP: usize = 32;
const TOPOLOGY: u8 = 1;
const APPEARANCE: u8 = 2;
const TIME: u8 = 4;
const TASKBAR: u8 = 8;
const FOREGROUND: u8 = 16;

/// Coalescing queue between callbacks and the message loop. Order-sensitive
/// signals (power, session, Explorer, shutdown) keep their sequence; idempotent
/// refreshes collapse into one flag each and are delivered after them.
#[derive(Debug, Default)]
pub struct Pending {
    flags: u8,
    ordered: std::collections::VecDeque<Signal>,
}
impl Pending {
    pub fn push(&mut self, signal: Signal) {
        match signal {
            Signal::TopologyChanged => self.flags |= TOPOLOGY,
            Signal::AppearanceMaybeChanged => self.flags |= APPEARANCE,
            Signal::TimeChanged => self.flags |= TIME,
            Signal::Shell(ShellEvent::TaskbarChanged) => self.flags |= TASKBAR,
            Signal::Shell(ShellEvent::ForegroundChanged) => self.flags |= FOREGROUND,
            ordered => {
                if self.ordered.len() == ORDERED_CAP {
                    // A flood of power/session notifications keeps the newest;
                    // each handler re-reads actual state, so none is lost.
                    self.ordered.pop_front();
                }
                self.ordered.push_back(ordered);
            }
        }
    }
    pub fn is_empty(&self) -> bool {
        self.flags == 0 && self.ordered.is_empty()
    }
    /// Drain everything: ordered signals first, then one of each refresh.
    pub fn take(&mut self) -> Vec<Signal> {
        let mut out: Vec<_> = self.ordered.drain(..).collect();
        for (bit, signal) in [
            (TOPOLOGY, Signal::TopologyChanged),
            (APPEARANCE, Signal::AppearanceMaybeChanged),
            (TIME, Signal::TimeChanged),
            (TASKBAR, Signal::Shell(ShellEvent::TaskbarChanged)),
            (FOREGROUND, Signal::Shell(ShellEvent::ForegroundChanged)),
        ] {
            if self.flags & bit != 0 {
                out.push(signal);
            }
        }
        self.flags = 0;
        out
    }
}

/// Bounded fast polling after a taskbar event: the slide animation produces a
/// few intermediate rectangles that events may not fully report.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TransitionPoll {
    remaining: u8,
    stable: u8,
}
impl TransitionPoll {
    pub fn begin(&mut self) {
        self.remaining = TRANSITION_POLL_LIMIT;
        self.stable = 0;
    }
    pub fn active(&self) -> bool {
        self.remaining > 0
    }
    /// Record a poll's outcome. Two consecutive unchanged polls end the burst.
    pub fn observed(&mut self, changed: bool) {
        self.remaining = self.remaining.saturating_sub(1);
        self.stable = if changed { 0 } else { self.stable + 1 };
        if self.stable >= 2 {
            self.remaining = 0;
        }
    }
    /// Next poll delay: fast while a transition is being tracked.
    pub fn interval(&self) -> Duration {
        if self.active() {
            TRANSITION_POLL
        } else {
            RECOVERY_POLL
        }
    }
}

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::{Notifications, WM_WAKE, drain, enqueue, handle_window_message};

#[cfg(test)]
mod tests {
    use super::raw::*;
    use super::*;

    const TASKBAR_CREATED: u32 = 0xC123;

    #[test]
    fn maps_notifications_to_signals() {
        let map = |m, w, s| signal_for_message(m, w, s, TASKBAR_CREATED);
        assert_eq!(map(TASKBAR_CREATED, 0, None), Some(Signal::TaskbarCreated));
        assert_eq!(map(WM_DISPLAYCHANGE, 32, None), Some(Signal::TopologyChanged));
        assert_eq!(map(WM_DPICHANGED, 0, None), Some(Signal::TopologyChanged));
        assert_eq!(map(WM_TIMECHANGE, 0, None), Some(Signal::TimeChanged));
        assert_eq!(
            map(WM_THEMECHANGED, 0, None),
            Some(Signal::AppearanceMaybeChanged)
        );
        assert_eq!(
            map(WM_SETTINGCHANGE, 0, Some("ImmersiveColorSet")),
            Some(Signal::AppearanceMaybeChanged)
        );
        assert_eq!(
            map(WM_SETTINGCHANGE, 0, Some("TraySettings")),
            Some(Signal::TopologyChanged)
        );
        assert_eq!(
            map(WM_SETTINGCHANGE, SPI_SETWORKAREA, None),
            Some(Signal::TopologyChanged)
        );
        assert_eq!(map(WM_POWERBROADCAST, PBT_APMSUSPEND, None), Some(Signal::Suspended));
        for resume in [PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND] {
            assert_eq!(map(WM_POWERBROADCAST, resume, None), Some(Signal::Resumed));
        }
        assert_eq!(map(WM_POWERBROADCAST, 0x0A, None), None);
        assert_eq!(
            map(WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK, None),
            Some(Signal::SessionLocked)
        );
        assert_eq!(
            map(WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK, None),
            Some(Signal::SessionUnlocked)
        );
        assert_eq!(
            map(WM_WTSSESSION_CHANGE, WTS_REMOTE_CONNECT, None),
            Some(Signal::TopologyChanged)
        );
        assert_eq!(map(WM_ENDSESSION, 0, None), None, "cancelled shutdown");
        assert_eq!(map(WM_ENDSESSION, 1, None), Some(Signal::EndSession));
        assert_eq!(map(0x0001, 0, None), None);
    }

    #[test]
    fn an_unregistered_taskbar_message_never_matches_zero() {
        assert_eq!(signal_for_message(0, 0, None, 0), None);
    }

    #[test]
    fn win_event_filter_ignores_unrelated_high_rate_objects() {
        let never = || -> bool { panic!("class lookup must be lazy for system events") };
        assert_eq!(
            shell_event_for(WinEventKind::Foreground, true, never),
            Some(ShellEvent::ForegroundChanged)
        );
        assert_eq!(
            shell_event_for(WinEventKind::MinimizeEnd, false, never),
            Some(ShellEvent::ForegroundChanged)
        );
        assert_eq!(
            shell_event_for(WinEventKind::ObjectLocationChange, true, || true),
            Some(ShellEvent::TaskbarChanged)
        );
        assert_eq!(
            shell_event_for(WinEventKind::ObjectLocationChange, true, || false),
            None
        );
        // Caret/cursor/child objects are rejected without a class lookup.
        assert_eq!(
            shell_event_for(WinEventKind::ObjectLocationChange, false, never),
            None
        );
    }

    #[test]
    fn bursts_collapse_to_one_refresh_and_order_sensitive_signals_keep_order() {
        let mut pending = Pending::default();
        assert!(pending.is_empty());
        for _ in 0..10 {
            pending.push(Signal::TopologyChanged);
            pending.push(Signal::Shell(ShellEvent::TaskbarChanged));
        }
        pending.push(Signal::SessionLocked);
        pending.push(Signal::TimeChanged);
        pending.push(Signal::SessionUnlocked);
        pending.push(Signal::TaskbarCreated);
        assert_eq!(
            pending.take(),
            vec![
                Signal::SessionLocked,
                Signal::SessionUnlocked,
                Signal::TaskbarCreated,
                Signal::TopologyChanged,
                Signal::TimeChanged,
                Signal::Shell(ShellEvent::TaskbarChanged),
            ]
        );
        assert!(pending.is_empty());
        assert!(pending.take().is_empty());
    }

    #[test]
    fn ordered_queue_is_bounded_and_keeps_the_newest() {
        let mut pending = Pending::default();
        for _ in 0..1_000 {
            pending.push(Signal::Suspended);
        }
        pending.push(Signal::Resumed);
        let taken = pending.take();
        assert_eq!(taken.len(), ORDERED_CAP);
        assert_eq!(taken.last(), Some(&Signal::Resumed));
    }

    #[test]
    fn transition_polling_is_bounded_and_ends_when_stable() {
        let mut poll = TransitionPoll::default();
        assert!(!poll.active());
        assert_eq!(poll.interval(), RECOVERY_POLL);
        poll.begin();
        assert_eq!(poll.interval(), TRANSITION_POLL);
        poll.observed(true);
        poll.observed(false);
        assert!(poll.active(), "one unchanged poll is not stable yet");
        poll.observed(false);
        assert!(!poll.active());
        assert_eq!(poll.interval(), RECOVERY_POLL);
        // A shell that never settles still stops after the limit.
        poll.begin();
        for _ in 0..TRANSITION_POLL_LIMIT {
            assert!(poll.active());
            poll.observed(true);
        }
        assert!(!poll.active());
    }
}
