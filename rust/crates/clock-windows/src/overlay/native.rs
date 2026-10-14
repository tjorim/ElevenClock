use super::may_show;
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
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
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

/// All windows/resources live on this UI thread. Worker channels contain only
/// value geometry and click indices. No blocking discovery or UIA in callbacks.
pub fn run(
    updates: Receiver<Update>,
    clicks: SyncSender<ExperimentSlot>,
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
        let started = Instant::now();
        let mut lease = Instant::now();
        let mut windows: Vec<Window> = Vec::new();
        let mut message = MSG::default();
        let mut wait_for_update = false;
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
                if !show {
                    let _ = ShowWindow(window.hwnd, SW_HIDE);
                    continue;
                }
                // Relative to the actual taskbar's preceding window. Never use
                // HWND_TOPMOST, parent into Explorer, or change Explorer styles.
                let preceding = GetWindow(window.taskbar, GW_HWNDPREV).unwrap_or(HWND_TOP);
                let insert = if preceding == window.hwnd || preceding.is_invalid() {
                    HWND_TOP
                } else {
                    preceding
                };
                let r = window.slot.clock;
                if SetWindowPos(
                    window.hwnd,
                    Some(insert),
                    r.left(),
                    r.top(),
                    r.width(),
                    r.height(),
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
                .is_err()
                {
                    let _ = ShowWindow(window.hwnd, SW_HIDE);
                }
            }
            if lease.elapsed() >= Duration::from_secs(8) {
                windows.clear();
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        // windows drop before class registration is released.
        windows.clear();
        UnregisterClassW(class.lpszClassName, Some(module.into()))?;
    }
    Ok(())
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match message {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_LBUTTONUP => {
                let _ = PostMessageW(Some(hwnd), WM_APP + 3, WPARAM(0), LPARAM(0));
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
