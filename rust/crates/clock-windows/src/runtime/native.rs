//! Windows platform layer and the single UI message loop (R08).
//!
//! Everything native lives on one thread: the hidden host window, the overlay
//! windows, Direct2D, timers, WinEvent hooks, session notifications and the
//! Settings dialog. The only other thread is the bounded discovery worker, which
//! owns helper *processes* and communicates by value.
//!
//! Window procedures and hook callbacks only record a flag or queue a `Signal`
//! (`events::enqueue`); the loop turns them into `Runtime` inputs after the
//! message returns, so callbacks stay short and cannot re-enter runtime state.
use super::{
    Input, Observation, PaintContext, PaintResult, Platform, Runtime, ShellIdentity,
    overlay_window::{Overlay, OverlayClass, REPAINT_EPOCH},
    shell::covers_monitor,
};
use crate::{
    clock_time,
    events::{self, Notifications, WM_WAKE},
    instance::{HOST_CLASS, InstanceGuard, LaunchMode, WM_SHOW_SETTINGS},
    preferences_store::{PreferenceSession, PreferencesStore},
    render::{PaintOutcome, PaintRequest, RenderError, Renderer, WorstCase},
    settings::{PlatformError, SettingsWindow},
    taskbar::{
        probe::{current_shell, physical_rect},
        worker::{DEADLINE, DiscoveryWorker, TEARDOWN_LIMIT, same_executable},
    },
    theme::{self, SystemAppearance},
};
use chrono::{DateTime, FixedOffset};
use clock_core::{
    PhysicalRect, ShellGeneration, TaskbarKey, TaskbarSnapshot, ValidatedPreferences,
};
use std::{
    cell::Cell,
    collections::HashMap,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow},
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    },
    core::HSTRING,
};

const WM_DISCOVERY: u32 = WM_APP + 3;
const TIMER_CLOCK: usize = 1;
const TIMER_POLL: usize = 2;

thread_local! {
    static CLOCK_TIMER: Cell<bool> = const { Cell::new(false) };
    static POLL_TIMER: Cell<bool> = const { Cell::new(false) };
    static DISCOVERY_READY: Cell<bool> = const { Cell::new(false) };
    static SHOW_SETTINGS: Cell<bool> = const { Cell::new(false) };
    static EXIT: Cell<bool> = const { Cell::new(false) };
    static PANICKED: Cell<bool> = const { Cell::new(false) };
}

/// Ask the loop to quit (Settings "Exit", `WM_CLOSE` on the host, tray later).
/// Quitting removes every overlay; the native clock was never modified.
pub(crate) fn request_exit() {
    EXIT.set(true);
}

fn render_error(error: RenderError) -> windows::core::Error {
    let code = match error {
        RenderError::Native { code, .. } => code,
        _ => 0x80004005u32 as i32,
    };
    windows::core::Error::new(windows::core::HRESULT(code), error.to_string())
}

fn is_taskbar_class(hwnd: HWND) -> bool {
    let mut class = [0u16; 32];
    // SAFETY: writable buffer of the advertised length; stale handles yield zero.
    let length = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
    let text = String::from_utf16_lossy(&class[..length]);
    text == "Shell_TrayWnd" || text == "Shell_SecondaryTrayWnd"
}
fn window_rect(hwnd: HWND) -> Option<PhysicalRect> {
    let mut rect = RECT::default();
    // SAFETY: live output buffer; a stale handle fails closed.
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    physical_rect(rect)
}

/// What is in front: its rectangle, and whether it is the desktop or the shell.
fn foreground() -> (Option<PhysicalRect>, bool) {
    // SAFETY: simple queries on a handle that may vanish at any time.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return (None, false);
    }
    let mut class = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
    let class = String::from_utf16_lossy(&class[..length]);
    let shell = matches!(
        class.as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    );
    (window_rect(hwnd), shell)
}

#[derive(Clone, Copy)]
struct TaskbarInfo {
    dpi: u32,
}
struct PaintState {
    preferences: ValidatedPreferences,
    worst_case: WorstCase,
    appearance: SystemAppearance,
}

pub struct NativePlatform {
    host: HWND,
    started: Instant,
    // Overlays first: windows are destroyed before the class guard unregisters.
    overlays: HashMap<TaskbarKey, Overlay>,
    info: HashMap<TaskbarKey, TaskbarInfo>,
    notifications: Option<Notifications>,
    worker: DiscoveryWorker,
    renderer: Renderer,
    _class: OverlayClass,
    shell: Option<ShellIdentity>,
    paint_state: Option<PaintState>,
    repaint_epoch: u64,
}

impl NativePlatform {
    fn taskbar_window(&self, key: TaskbarKey) -> Option<HWND> {
        let hwnd = HWND(self.worker.handle_for(key)? as usize as *mut _);
        // The handle came from another process' report: revalidate that it is
        // still a taskbar owned by the current shell before every use.
        let mut pid = 0;
        // SAFETY: read-only queries on a possibly stale handle.
        let valid = unsafe {
            IsWindow(Some(hwnd)).as_bool() && {
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                true
            }
        };
        (valid && is_taskbar_class(hwnd) && self.shell.is_some_and(|s| s.pid == pid))
            .then_some(hwnd)
    }

    fn remember(&mut self, context: &PaintContext<'_>) {
        let unchanged = self.paint_state.as_ref().is_some_and(|s| {
            s.preferences == *context.preferences
                && s.appearance == context.appearance
                && s.worst_case.fingerprint() == context.worst_case.fingerprint()
        });
        if !unchanged {
            self.paint_state = Some(PaintState {
                preferences: context.preferences.clone(),
                worst_case: context.worst_case.clone(),
                appearance: context.appearance,
            });
        }
    }

    /// Pixels were invalidated by Windows (uncovered, DPI or display change):
    /// redraw every shown overlay from its last text, with no reducer involvement.
    fn repaint_invalidated(&mut self) {
        let epoch = REPAINT_EPOCH.load(Ordering::Relaxed);
        if epoch == self.repaint_epoch {
            return;
        }
        self.repaint_epoch = epoch;
        let Some(state) = &self.paint_state else {
            return;
        };
        let keys: Vec<_> = self.overlays.keys().copied().collect();
        for key in keys {
            let (Some(overlay), Some(info)) = (self.overlays.get_mut(&key), self.info.get(&key))
            else {
                continue;
            };
            let Some(text) = overlay.last_text.clone() else {
                continue;
            };
            if !overlay.is_shown() {
                continue;
            }
            overlay.surface.mark_dirty();
            let request = PaintRequest {
                text: &text,
                worst_case: &state.worst_case,
                dpi: info.dpi,
                font_pt: state.preferences.font_size_pt(),
                palette: theme::resolve(state.preferences.theme(), state.appearance),
            };
            if !matches!(
                overlay.paint(&self.renderer, &request),
                Ok(PaintOutcome::Painted | PaintOutcome::Unchanged)
            ) {
                overlay.hide();
            }
        }
    }
}

impl Platform for NativePlatform {
    fn monotonic(&mut self) -> Duration {
        self.started.elapsed()
    }
    fn local_now(&mut self) -> DateTime<FixedOffset> {
        clock_time::local_now()
    }
    fn system_appearance(&mut self) -> SystemAppearance {
        theme::system_appearance()
    }
    fn refresh_fonts(&mut self) -> bool {
        self.renderer.refresh_system_font()
    }
    fn shell_identity(&mut self) -> Option<ShellIdentity> {
        current_shell().map(|(pid, started)| ShellIdentity { pid, started })
    }
    fn shell_changed(&mut self, shell: Option<ShellIdentity>) {
        self.shell = shell;
        if let Some(notifications) = &mut self.notifications {
            notifications.retarget(shell.map_or(0, |s| s.pid));
        }
    }
    fn request_discovery(&mut self, generation: ShellGeneration) {
        self.worker.request(generation);
    }
    fn note_taskbars(&mut self, snapshots: &[TaskbarSnapshot]) {
        self.info = snapshots
            .iter()
            .map(|s| (s.key, TaskbarInfo { dpi: s.dpi }))
            .collect();
    }
    fn observe(&mut self, taskbar: &TaskbarSnapshot) -> Observation {
        let Some(hwnd) = self.taskbar_window(taskbar.key) else {
            return Observation::GONE;
        };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: read-only queries; the monitor handle is used immediately.
        let (monitor_rect, dpi, visible) = unsafe {
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
            let rect = (!monitor.is_invalid() && GetMonitorInfoW(monitor, &mut info).as_bool())
                .then(|| physical_rect(info.rcMonitor))
                .flatten();
            (rect, GetDpiForWindow(hwnd), IsWindowVisible(hwnd).as_bool())
        };
        let (front, front_is_shell) = foreground();
        Observation {
            exists: true,
            visible,
            rect: window_rect(hwnd),
            dpi,
            monitor_rect,
            fullscreen: monitor_rect
                .is_some_and(|monitor| covers_monitor(front, front_is_shell, monitor)),
        }
    }
    fn create_overlay(&mut self, key: TaskbarKey, bounds: PhysicalRect) -> Result<(), String> {
        let taskbar = self
            .taskbar_window(key)
            .ok_or_else(|| "taskbar window is no longer current".to_string())?;
        let overlay =
            Overlay::create(&self.renderer, taskbar, bounds).map_err(|e| e.to_string())?;
        self.overlays.insert(key, overlay);
        Ok(())
    }
    fn set_bounds(&mut self, key: TaskbarKey, bounds: PhysicalRect) -> Result<(), String> {
        self.overlays
            .get_mut(&key)
            .ok_or_else(|| "no overlay window".to_string())?
            .set_bounds(bounds)
            .map_err(|e| e.to_string())
    }
    fn destroy_overlay(&mut self, key: TaskbarKey) {
        self.overlays.remove(&key);
    }
    fn paint(&mut self, key: TaskbarKey, context: &PaintContext<'_>) -> PaintResult {
        let Some(dpi) = self.info.get(&key).map(|i| i.dpi) else {
            return PaintResult::Failed("taskbar metadata unavailable".into());
        };
        let Some(overlay) = self.overlays.get_mut(&key) else {
            return PaintResult::Failed("no overlay window".into());
        };
        let request = PaintRequest {
            text: context.text,
            worst_case: context.worst_case,
            dpi,
            font_pt: context.preferences.font_size_pt(),
            palette: theme::resolve(context.preferences.theme(), context.appearance),
        };
        let result = match overlay.paint(&self.renderer, &request) {
            Ok(PaintOutcome::Painted | PaintOutcome::Unchanged) => {
                overlay.last_text = Some(context.text.clone());
                // Expose only after correct pixels exist; re-anchor if needed.
                if !overlay.is_shown() || !overlay.is_anchored() {
                    match overlay.show_anchored() {
                        Ok(()) => PaintResult::Painted,
                        Err(error) => PaintResult::Failed(format!("placement: {error}")),
                    }
                } else {
                    PaintResult::Painted
                }
            }
            Ok(PaintOutcome::Overflow(overflow)) => {
                overlay.hide();
                PaintResult::Overflow(overflow)
            }
            Err(error) => {
                overlay.hide();
                PaintResult::Failed(error.to_string())
            }
        };
        self.remember(context);
        result
    }
    fn hide(&mut self, key: TaskbarKey) {
        if let Some(overlay) = self.overlays.get_mut(&key) {
            overlay.hide();
        }
    }
    fn maintain(&mut self) {
        for overlay in self.overlays.values_mut() {
            if overlay.is_shown() && !overlay.is_anchored() && overlay.show_anchored().is_err() {
                overlay.hide();
            }
        }
    }
    fn arm_timer(&mut self, delay: Duration) {
        set_timer(self.host, TIMER_CLOCK, delay);
    }
    fn arm_poll(&mut self, delay: Duration) {
        set_timer(self.host, TIMER_POLL, delay);
    }
    fn stop(&mut self) {
        // SAFETY: kills this window's own timers.
        unsafe {
            let _ = KillTimer(Some(self.host), TIMER_CLOCK);
            let _ = KillTimer(Some(self.host), TIMER_POLL);
        }
        self.overlays.clear();
        self.notifications = None;
        self.worker.shutdown(TEARDOWN_LIMIT * 2);
    }
}

fn set_timer(host: HWND, id: usize, delay: Duration) {
    // USER timers are 1 ms..~24 days; the minimum also bounds how tight a loop a
    // boundary-aligned wait can become.
    let millis = delay.as_millis().clamp(1, 0x7FFF_FFFF) as u32;
    // SAFETY: replaces this window's timer with the same id.
    unsafe {
        SetTimer(Some(host), id, millis, None);
    }
}

fn guard(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_TIMER => {
            // One-shot semantics on top of the periodic USER timer.
            // SAFETY: kills this window's own timer.
            unsafe {
                let _ = KillTimer(Some(hwnd), wparam.0);
            }
            match wparam.0 {
                TIMER_CLOCK => CLOCK_TIMER.set(true),
                TIMER_POLL => POLL_TIMER.set(true),
                _ => {}
            }
            LRESULT(0)
        }
        WM_DISCOVERY => {
            DISCOVERY_READY.set(true);
            LRESULT(0)
        }
        WM_SHOW_SETTINGS => {
            SHOW_SETTINGS.set(true);
            LRESULT(0)
        }
        WM_WAKE => LRESULT(0),
        WM_QUERYENDSESSION => LRESULT(1),
        // A polite external close (taskkill without /f) is an Exit request.
        WM_CLOSE => {
            request_exit();
            LRESULT(0)
        }
        _ => {
            if events::handle_window_message(message, wparam, lparam) {
                LRESULT(0)
            } else {
                // SAFETY: default handling for our own window.
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
    }
}

unsafe extern "system" fn host_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Never unwind through the system's frames: a panic becomes a clean shutdown.
    std::panic::catch_unwind(|| guard(hwnd, message, wparam, lparam)).unwrap_or_else(|_| {
        PANICKED.set(true);
        LRESULT(0)
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitReason {
    /// Exit chosen by the user, a polite close, or session end.
    Requested,
}

struct HostWindow {
    hwnd: HWND,
    instance: windows::Win32::Foundation::HINSTANCE,
}
impl HostWindow {
    fn create(title: &str) -> windows::core::Result<Self> {
        // SAFETY: registers a class and creates a hidden top-level (not
        // message-only) window, so broadcast messages reach it. Never shown.
        unsafe {
            let instance: windows::Win32::Foundation::HINSTANCE = GetModuleHandleW(None)?.into();
            // The class name must outlive RegisterClassW, so bind it first.
            let class_name = HSTRING::from(HOST_CLASS);
            let class = WNDCLASSW {
                lpfnWndProc: Some(host_proc),
                hInstance: instance,
                lpszClassName: windows::core::PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(windows::core::Error::from_win32());
            }
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                &HSTRING::from(HOST_CLASS),
                &HSTRING::from(title),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            );
            match hwnd {
                Ok(hwnd) => Ok(Self { hwnd, instance }),
                Err(error) => {
                    let _ = UnregisterClassW(&HSTRING::from(HOST_CLASS), Some(instance));
                    Err(error)
                }
            }
        }
    }
}
impl Drop for HostWindow {
    fn drop(&mut self) {
        // SAFETY: destroys our window, then releases its class registration.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let _ = UnregisterClassW(&HSTRING::from(HOST_CLASS), Some(self.instance));
        }
    }
}

fn start_error(error: windows::core::Error) -> PlatformError {
    PlatformError::new("ElevenClock could not start its clock", error)
}

/// Run the application until Exit. Returns after every overlay, hook, timer,
/// helper and window has been released; the native clock is untouched throughout.
pub fn run<S: PreferencesStore>(
    session: PreferenceSession<S>,
    launch: LaunchMode,
    instance: &InstanceGuard,
) -> Result<ExitReason, PlatformError> {
    let settings = SettingsWindow::new()?;
    let host = HostWindow::create(&instance.names().window_title).map_err(start_error)?;
    let renderer = Renderer::new().map_err(|e| start_error(render_error(e)))?;
    let class = OverlayClass::register().map_err(start_error)?;
    let notifications = Notifications::install(host.hwnd).map_err(start_error)?;
    let target = host.hwnd.0 as usize;
    let factory = same_executable().map_err(|e| start_error(e.into()))?;
    let worker = DiscoveryWorker::spawn(
        factory,
        DEADLINE,
        Box::new(move || {
            // SAFETY: posting to the host window from the worker thread is the
            // documented thread-safe way to wake a UI thread; failure only delays
            // delivery until the next poll.
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(target as *mut _)),
                    WM_DISCOVERY,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }),
    )
    .map_err(|e| start_error(e.into()))?;
    let platform = NativePlatform {
        host: host.hwnd,
        started: Instant::now(),
        overlays: HashMap::new(),
        info: HashMap::new(),
        notifications: Some(notifications),
        worker,
        renderer,
        _class: class,
        shell: None,
        paint_state: None,
        repaint_epoch: REPAINT_EPOCH.load(Ordering::Relaxed),
    };
    let mut runtime = Runtime::new(platform, session.active().clone());
    runtime.start();
    if launch == LaunchMode::Interactive {
        settings.show_or_focus()?;
    }

    let mut message = MSG::default();
    let result = loop {
        // SAFETY: standard loop on the thread that owns every window.
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if status.0 <= 0 {
            break Ok(ExitReason::Requested);
        }
        if !settings.handle_message(&mut message) {
            // SAFETY: dispatches to our window procedures on this thread.
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if let Err(error) = pump(&mut runtime, &settings) {
            break Err(error);
        }
        if runtime.is_finished() {
            break Ok(ExitReason::Requested);
        }
    };

    // Teardown order matters: overlays and hooks go before their window/class,
    // and the helper is reaped. This runs on every exit path.
    runtime.handle(Input::Quit);
    settings.close();
    drop(runtime);
    drop(host);
    result
}

/// Convert recorded flags and queued signals into runtime inputs, outside any
/// callback. Returns an error only for an internal fault (a panic in a callback).
fn pump(
    runtime: &mut Runtime<NativePlatform>,
    settings: &SettingsWindow,
) -> Result<(), PlatformError> {
    if PANICKED.take() {
        // Fail closed: the clock overlay is removed, the native clock remains.
        request_exit();
        return Err(start_error(windows::core::Error::new(
            windows::core::HRESULT(0x80004005u32 as i32),
            "An internal error occurred; ElevenClock stopped and the Windows clock is shown.",
        )));
    }
    if EXIT.take() {
        runtime.handle(Input::Quit);
        return Ok(());
    }
    if DISCOVERY_READY.take() {
        for report in runtime.platform().worker.take_reports() {
            runtime.handle(Input::Discovery(report));
        }
    }
    if CLOCK_TIMER.take() {
        runtime.handle(Input::Timer);
    }
    if POLL_TIMER.take() {
        runtime.handle(Input::Poll);
    }
    for signal in events::drain() {
        runtime.handle(Input::Signal(signal));
    }
    runtime.platform_mut().repaint_invalidated();
    if SHOW_SETTINGS.take() {
        settings.show_or_focus()?;
    }
    Ok(())
}
