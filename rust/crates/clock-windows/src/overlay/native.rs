use super::{BandAction, band_action, may_show};
use clock_core::PhysicalRect;
use std::{
    sync::mpsc::{Receiver, SyncSender},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{LibraryLoader::GetModuleHandleW, Threading::*},
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{GetCapture, ReleaseCapture, SetCapture},
            WindowsAndMessaging::*,
        },
    },
    core::{Result, w},
};

#[derive(Clone, Debug, PartialEq)]
pub struct ExperimentSlot {
    pub clock: PhysicalRect,
    pub taskbar: PhysicalRect,
    pub monitor: PhysicalRect,
    pub dpi: u32,
    pub shell_pid: u32,
    pub shell_started: u64,
    pub primary: bool,
}
pub enum Update {
    Slots(Vec<ExperimentSlot>),
    Failed,
    InteractionComplete,
}
struct WindowClass {
    instance: HINSTANCE,
    name: windows::core::PCWSTR,
}
impl Drop for WindowClass {
    fn drop(&mut self) {
        // All owned windows are declared after this guard and drop first.
        unsafe {
            let _ = UnregisterClassW(self.name, Some(self.instance));
        }
    }
}
struct Window {
    hwnd: HWND,
    taskbar: HWND,
    slot: ExperimentSlot,
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn bounds(hwnd: HWND) -> Option<PhysicalRect> {
    let mut r = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &mut r).ok()?;
    }
    crate::taskbar::probe::physical_rect(r)
}
fn shell_current(hwnd: HWND, slot: &ExperimentSlot) -> bool {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != slot.shell_pid {
            return false;
        }
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let (mut creation, mut exit, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let result = GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user);
        let _ = CloseHandle(process);
        result.is_ok()
            && (u64::from(creation.dwHighDateTime) << 32 | u64::from(creation.dwLowDateTime))
                == slot.shell_started
    }
}
fn taskbar(slot: &ExperimentSlot) -> Result<HWND> {
    for name in [w!("Shell_TrayWnd"), w!("Shell_SecondaryTrayWnd")] {
        let mut after = None;
        for _ in 0..128 {
            let Ok(hwnd) = (unsafe { FindWindowExW(None, after, name, None) }) else {
                break;
            };
            if bounds(hwnd) == Some(slot.taskbar) && shell_current(hwnd, slot) {
                return Ok(hwnd);
            }
            after = Some(hwnd);
        }
    }
    Err(windows::core::Error::from_win32())
}

/// Revalidate the native epoch immediately before an interaction, including
/// creation time so PID reuse cannot authorize an old click.
pub(crate) fn check_slot(slot: &ExperimentSlot) -> Result<()> {
    let hwnd = taskbar(slot)?;
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        if GetDpiForWindow(hwnd) != slot.dpi
            || !IsWindowVisible(hwnd).as_bool()
            || !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL), &mut info).as_bool()
            || crate::taskbar::probe::physical_rect(info.rcMonitor) != Some(slot.monitor)
        {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x80004005u32 as i32),
                "Native interaction epoch changed",
            ));
        }
    }
    Ok(())
}

/// All windows/resources live on this UI thread. Worker channels contain only
/// value geometry and click indices. No blocking discovery or UIA in callbacks.
pub fn run(
    updates: Receiver<Update>,
    clicks: SyncSender<ExperimentSlot>,
    observations: SyncSender<String>,
    duration: Duration,
) -> Result<()> {
    unsafe {
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: module.into(),
            lpszClassName: w!("ElevenClock.R03.Overlay"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_win32());
        }
        let _class_owner = WindowClass {
            instance: module.into(),
            name: class.lpszClassName,
        };
        let started = Instant::now();
        let mut lease = Instant::now();
        let mut windows: Vec<Window> = Vec::new();
        let mut message = MSG::default();
        let mut wait_for_update = false;
        let mut last_observation = String::new();
        while started.elapsed() < duration {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                if message.message == WM_QUIT {
                    return Ok(());
                }
                if message.message == WM_APP + 3 && !wait_for_update {
                    let slot = windows
                        .iter()
                        .find(|w| w.hwnd == message.hwnd)
                        .map(|w| w.slot.clone());
                    // Remove coverage before provider interaction. Busy clicks
                    // cannot enqueue repeated actions; no mouse relay.
                    for window in &windows {
                        let _ = ShowWindow(window.hwnd, SW_HIDE);
                    }
                    wait_for_update = true;
                    if let Some(slot) = slot {
                        let _ = clicks.try_send(slot);
                    }
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            while let Ok(update) = updates.try_recv() {
                windows.clear();
                if matches!(update, Update::InteractionComplete) {
                    wait_for_update = false;
                }
                lease = Instant::now();
                if let Update::Slots(slots) = update {
                    if wait_for_update {
                        continue;
                    }
                    for slot in slots {
                        let bar = taskbar(&slot)?;
                        let r = slot.clock;
                        let hwnd = CreateWindowExW(
                            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                            class.lpszClassName,
                            w!("2642.2"),
                            WS_POPUP,
                            r.left(),
                            r.top(),
                            r.width(),
                            r.height(),
                            None,
                            None,
                            Some(module.into()),
                            None,
                        )?;
                        windows.push(Window {
                            hwnd,
                            taskbar: bar,
                            slot,
                        });
                    }
                }
            }
            let fg = GetForegroundWindow();
            let fg_bounds = bounds(fg);
            let mut fg_class = [0u16; 128];
            let n = GetClassNameW(fg, &mut fg_class);
            let fg_class = String::from_utf16_lossy(&fg_class[..n.max(0) as usize]);
            let shell = matches!(
                fg_class.as_str(),
                "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
            );
            for window in &windows {
                let current = (shell || fg_bounds.is_some())
                    && !wait_for_update
                    && lease.elapsed() < Duration::from_secs(8)
                    && shell_current(window.taskbar, &window.slot)
                    && GetDpiForWindow(window.taskbar) == window.slot.dpi;
                let show = bounds(window.taskbar).is_some_and(|r| {
                    may_show(
                        current,
                        IsWindowVisible(window.taskbar).as_bool(),
                        r,
                        window.slot.taskbar,
                        window.slot.monitor,
                        fg_bounds,
                        shell,
                    )
                });
                let state = format!(
                    "show={show}; current={current}; shell_current={}; taskbar_visible={}; taskbar_dpi={}; expected_dpi={}; foreground={fg_bounds:?}; taskbar={:?}",
                    shell_current(window.taskbar, &window.slot),
                    IsWindowVisible(window.taskbar).as_bool(),
                    GetDpiForWindow(window.taskbar),
                    window.slot.dpi,
                    bounds(window.taskbar)
                );
                if state != last_observation {
                    let _ = observations.try_send(state.clone());
                    last_observation = state;
                }
                let root_topmost =
                    GetWindowLongPtrW(window.taskbar, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
                let own_topmost =
                    GetWindowLongPtrW(window.hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
                let band = band_action(show, root_topmost, own_topmost);
                if band != BandAction::Preserve {
                    // Hide before band changes: temporary promotion is never
                    // exposed above unrelated topmost windows. Only our HWND
                    // changes, and hidden/fullscreen coverage is demoted.
                    let _ = ShowWindow(window.hwnd, SW_HIDE);
                    let target = if band == BandAction::PromoteToTaskbar {
                        HWND_TOPMOST
                    } else {
                        HWND_NOTOPMOST
                    };
                    if SetWindowPos(
                        window.hwnd,
                        Some(target),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    )
                    .is_err()
                    {
                        continue;
                    }
                }
                if !show {
                    let _ = ShowWindow(window.hwnd, SW_HIDE);
                    continue;
                }
                // Now insert immediately before the taskbar in its actual band.
                // Do not raise an already adjacent overlay above other apps.
                let preceding = GetWindow(window.taskbar, GW_HWNDPREV).unwrap_or(HWND_TOP);
                let adjacent = preceding == window.hwnd;
                let insert = if preceding.is_invalid() || adjacent {
                    HWND_TOP
                } else {
                    preceding
                };
                let preserve_order = if adjacent {
                    SWP_NOZORDER
                } else {
                    SET_WINDOW_POS_FLAGS(0)
                };
                let r = window.slot.clock;
                let positioned = SetWindowPos(
                    window.hwnd,
                    Some(insert),
                    r.left(),
                    r.top(),
                    r.width(),
                    r.height(),
                    SWP_NOACTIVATE | SWP_SHOWWINDOW | preserve_order,
                );
                if let Err(error) = positioned {
                    let _ =
                        observations.try_send(format!("SetWindowPos failed: {}", error.code().0));
                    let _ = ShowWindow(window.hwnd, SW_HIDE);
                } else if !IsWindowVisible(window.hwnd).as_bool() {
                    let _ = observations
                        .try_send("SetWindowPos succeeded but window is not visible".into());
                } else {
                    // Paint on the owning thread before exposing an unpainted
                    // surface to the capture harness or the next message tick.
                    let _ = UpdateWindow(window.hwnd);
                }
            }
            if lease.elapsed() >= Duration::from_secs(8) {
                windows.clear();
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        // windows drop before class registration is released.
        windows.clear();
    }
    Ok(())
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match message {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_LBUTTONDOWN => {
                // GWLP_USERDATA is an owned scalar press flag, never a pointer
                // to borrowed state. Capture keeps the release on this window.
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 1);
                SetCapture(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let pressed = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) == 1;
                if GetCapture() == hwnd {
                    let _ = ReleaseCapture();
                }
                let mut point = POINT::default();
                let mut rect = RECT::default();
                if pressed
                    && IsWindowVisible(hwnd).as_bool()
                    && GetCursorPos(&mut point).is_ok()
                    && GetWindowRect(hwnd, &mut rect).is_ok()
                    && point.x >= rect.left
                    && point.x < rect.right
                    && point.y >= rect.top
                    && point.y < rect.bottom
                {
                    let _ = PostMessageW(Some(hwnd), WM_APP + 3, WPARAM(0), LPARAM(0));
                }
                LRESULT(0)
            }
            WM_CANCELMODE | WM_CAPTURECHANGED => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                if message == WM_CANCELMODE && GetCapture() == hwnd {
                    let _ = ReleaseCapture();
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_DPICHANGED | WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                let _ = ShowWindow(hwnd, SW_HIDE);
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut r = RECT::default();
                let _ = GetClientRect(hwnd, &mut r);
                let _ = FillRect(dc, &r, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                let _ = SetBkMode(dc, TRANSPARENT);
                let _ = SetTextColor(dc, COLORREF(0xffffff));
                let height = -((12 * GetDpiForWindow(hwnd) / 96) as i32);
                let font = CreateFontW(
                    height,
                    0,
                    0,
                    0,
                    400,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    DEFAULT_QUALITY,
                    DEFAULT_PITCH.0 as u32,
                    w!("Segoe UI"),
                );
                let old = SelectObject(dc, HGDIOBJ(font.0));
                let mut text = "2642.2".encode_utf16().collect::<Vec<_>>();
                let _ = DrawTextW(
                    dc,
                    &mut text,
                    &mut r,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
                SelectObject(dc, old);
                let _ = DeleteObject(HGDIOBJ(font.0));
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    }
}
