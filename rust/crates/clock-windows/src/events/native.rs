//! Windows registration for [`super::Signal`] sources. Everything here runs on
//! the single UI thread: window procedures and out-of-context WinEvent callbacks
//! are delivered by its message loop, so a thread-local queue needs no locking.
use super::{Pending, Signal, WinEventKind, shell_event_for, signal_for_message};
use std::cell::{Cell, RefCell};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        System::RemoteDesktop::{
            NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
        },
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            WindowsAndMessaging::{
                EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_SHOW,
                EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MOVESIZEEND,
                GetClassNameW, OBJID_WINDOW, PostMessageW, RegisterWindowMessageW,
                WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP,
            },
        },
    },
    core::{PCWSTR, Result, w},
};

/// Posted to the host window when the queue becomes non-empty, so a callback
/// that ran inside the message wait still wakes the loop.
pub const WM_WAKE: u32 = WM_APP + 1;

thread_local! {
    static QUEUE: RefCell<Pending> = RefCell::new(Pending::default());
    static WAKE_TARGET: Cell<isize> = const { Cell::new(0) };
    static WAKE_POSTED: Cell<bool> = const { Cell::new(false) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

/// Queue a signal and make sure the loop will look at it. Allocation-light and
/// non-blocking: safe to call from a window procedure or hook callback.
pub fn enqueue(signal: Signal) {
    QUEUE.with(|queue| queue.borrow_mut().push(signal));
    if !WAKE_POSTED.replace(true) {
        let target = WAKE_TARGET.get();
        if target != 0 {
            // SAFETY: the target is the host window, destroyed only after the
            // loop ends. A failed post is recovered by the next message or poll.
            let posted = unsafe {
                PostMessageW(
                    Some(HWND(target as *mut _)),
                    WM_WAKE,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
            if posted.is_err() {
                WAKE_POSTED.set(false);
            }
        }
    }
}

/// Everything queued since the last call, coalesced and ordered.
pub fn drain() -> Vec<Signal> {
    WAKE_POSTED.set(false);
    QUEUE.with(|queue| queue.borrow_mut().take())
}

/// Map and queue a window message. Returns whether it was a runtime notification.
/// Called from the host window procedure; `lparam` for `WM_SETTINGCHANGE` is a
/// NUL-terminated string valid for the duration of the message.
pub fn handle_window_message(message: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    let setting = (message == super::raw::WM_SETTINGCHANGE && lparam.0 != 0)
        // SAFETY: documented WM_SETTINGCHANGE payload, valid during the message.
        .then(|| unsafe { PCWSTR(lparam.0 as *const u16).to_string() }.unwrap_or_default());
    match signal_for_message(message, wparam.0, setting.as_deref(), TASKBAR_CREATED.get()) {
        Some(signal) => {
            enqueue(signal);
            true
        }
        None => false,
    }
}

fn is_taskbar(hwnd: HWND) -> bool {
    let mut class = [0u16; 32];
    // SAFETY: the buffer is writable for its advertised length; stale handles
    // simply return zero characters.
    let length = unsafe { GetClassNameW(hwnd, &mut class) };
    let class = &class[..length.max(0) as usize];
    class == "Shell_TrayWnd".encode_utf16().collect::<Vec<_>>()
        || class == "Shell_SecondaryTrayWnd".encode_utf16().collect::<Vec<_>>()
}

/// Delivered on the installing thread by its message loop (out of context), so
/// no foreign code runs in other processes. Keep this short: classify and queue.
unsafe extern "system" fn win_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    let kind = match event {
        EVENT_SYSTEM_FOREGROUND => WinEventKind::Foreground,
        EVENT_SYSTEM_MINIMIZEEND => WinEventKind::MinimizeEnd,
        EVENT_SYSTEM_MOVESIZEEND => WinEventKind::MoveSizeEnd,
        EVENT_OBJECT_SHOW => WinEventKind::ObjectShow,
        EVENT_OBJECT_HIDE => WinEventKind::ObjectHide,
        EVENT_OBJECT_LOCATIONCHANGE => WinEventKind::ObjectLocationChange,
        _ => return,
    };
    let is_window = object == OBJID_WINDOW.0 && child == 0;
    if let Some(shell) = shell_event_for(kind, is_window, || is_taskbar(hwnd)) {
        enqueue(Signal::Shell(shell));
    }
}

/// Owns the registrations that make the runtime event driven. Dropping it
/// unhooks every WinEvent hook and unregisters session notifications, leaving
/// nothing for the process to leak on any exit path.
pub struct Notifications {
    host: HWND,
    session: bool,
    system_hooks: Vec<HWINEVENTHOOK>,
    object_hooks: Vec<HWINEVENTHOOK>,
    object_pid: u32,
}

impl Notifications {
    /// Registers `TaskbarCreated`, session notifications and the system-wide
    /// foreground hooks. Object hooks are added by [`Self::retarget`].
    pub fn install(host: HWND) -> Result<Self> {
        // SAFETY: documented registration calls with a live window and constant strings.
        unsafe {
            let message = RegisterWindowMessageW(w!("TaskbarCreated"));
            if message == 0 {
                return Err(windows::core::Error::from_win32());
            }
            TASKBAR_CREATED.set(message);
        }
        WAKE_TARGET.set(host.0 as isize);
        let mut this = Self {
            host,
            session: false,
            system_hooks: Vec::new(),
            object_hooks: Vec::new(),
            object_pid: 0,
        };
        // SAFETY: host is the live message window; failure is non-fatal because
        // the recovery poll observes lock state indirectly through unlock events
        // being absent; the runtime reports reduced event coverage.
        this.session =
            unsafe { WTSRegisterSessionNotification(host, NOTIFY_FOR_THIS_SESSION) }.is_ok();
        this.system_hooks.extend(hook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_MINIMIZEEND,
            0,
        ));
        Ok(this)
    }

    pub fn session_events_available(&self) -> bool {
        self.session
    }

    /// Scope the high-rate object hooks to the shell process. A different
    /// process id (Explorer restart) replaces them; zero removes them.
    pub fn retarget(&mut self, shell_pid: u32) {
        if self.object_pid == shell_pid && (shell_pid == 0 || !self.object_hooks.is_empty()) {
            return;
        }
        for handle in self.object_hooks.drain(..) {
            // SAFETY: each handle came from SetWinEventHook and is unhooked once.
            let _ = unsafe { UnhookWinEvent(handle) };
        }
        self.object_pid = shell_pid;
        if shell_pid != 0 {
            self.object_hooks
                .extend(hook(EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE, shell_pid));
            self.object_hooks.extend(hook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                shell_pid,
            ));
        }
    }

    /// Hook coverage for diagnostics; fewer hooks means more reliance on polling.
    pub fn hook_count(&self) -> usize {
        self.system_hooks.len() + self.object_hooks.len()
    }
}

fn hook(min: u32, max: u32, pid: u32) -> Option<HWINEVENTHOOK> {
    // SAFETY: out-of-context hook with a plain callback; installed and removed
    // on the UI thread, whose message loop delivers events.
    let handle = unsafe {
        SetWinEventHook(
            min,
            max,
            None,
            Some(win_event),
            pid,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    (!handle.is_invalid()).then_some(handle)
}

impl Drop for Notifications {
    fn drop(&mut self) {
        for handle in self.system_hooks.drain(..).chain(self.object_hooks.drain(..)) {
            // SAFETY: handles are owned and unhooked exactly once.
            let _ = unsafe { UnhookWinEvent(handle) };
        }
        if self.session {
            // SAFETY: matches the successful registration for this window.
            let _ = unsafe { WTSUnRegisterSessionNotification(self.host) };
        }
        WAKE_TARGET.set(0);
    }
}

#[cfg(test)]
mod tests {
    use super::super::raw::*;
    use windows::Win32::UI::WindowsAndMessaging as w;

    #[test]
    fn raw_constants_match_the_windows_definitions() {
        assert_eq!(WM_SYSCOLORCHANGE, w::WM_SYSCOLORCHANGE);
        assert_eq!(WM_SETTINGCHANGE, w::WM_SETTINGCHANGE);
        assert_eq!(WM_TIMECHANGE, w::WM_TIMECHANGE);
        assert_eq!(WM_ENDSESSION, w::WM_ENDSESSION);
        assert_eq!(WM_DISPLAYCHANGE, w::WM_DISPLAYCHANGE);
        assert_eq!(WM_POWERBROADCAST, w::WM_POWERBROADCAST);
        assert_eq!(WM_DPICHANGED, w::WM_DPICHANGED);
        assert_eq!(WM_THEMECHANGED, w::WM_THEMECHANGED);
        assert_eq!(WM_WTSSESSION_CHANGE, w::WM_WTSSESSION_CHANGE);
        assert_eq!(PBT_APMSUSPEND, w::PBT_APMSUSPEND as usize);
        assert_eq!(PBT_APMRESUMESUSPEND, w::PBT_APMRESUMESUSPEND as usize);
        assert_eq!(PBT_APMRESUMEAUTOMATIC, w::PBT_APMRESUMEAUTOMATIC as usize);
        assert_eq!(SPI_SETWORKAREA, 0x002F);
    }
}
