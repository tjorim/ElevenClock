//! Blocking development probe; run only inside an isolated helper, never in a
//! window callback. This inventory makes no clock-only claim.
use clock_core::*;
use std::{mem::size_of, time::Instant};
use windows::{
    Win32::{
        Foundation::{CloseHandle, FILETIME, HWND, LPARAM, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow},
        System::{
            Registry::*,
            Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        },
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    },
    core::{BOOL, w},
};

#[derive(Debug)]
pub struct NativeChild {
    pub class: String,
    /// `None` both when the query failed and when the window has no area.
    pub rect: Option<PhysicalRect>,
    /// `GetWindowRect` succeeded. A measured zero-area window has no hit target,
    /// so only an unmeasured child leaves the exclusion inventory incomplete.
    pub measured: bool,
    pub visible: bool,
}

#[derive(Debug)]
pub struct TaskbarObservation {
    pub snapshot: TaskbarSnapshot,
    /// The helper-local window handle. Only the UI thread's native layer keeps it,
    /// and only after revalidating class and owning process.
    pub hwnd: usize,
    pub shell_pid: u32,
    pub shell_started: u64,
    pub native_children: Vec<NativeChild>,
    pub native_inventory_incomplete: bool,
    pub uia: crate::taskbar::uia::Inventory,
    pub elapsed_micros: u128,
}

pub fn physical_rect(rect: RECT) -> Option<PhysicalRect> {
    PhysicalRect::new(
        rect.left,
        rect.top,
        rect.right.checked_sub(rect.left)?,
        rect.bottom.checked_sub(rect.top)?,
    )
    .ok()
}

fn class(hwnd: HWND) -> String {
    let mut buffer = [0u16; 256];
    // SAFETY: buffer is writable for its advertised capacity.
    let count = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..count.max(0) as usize])
}

fn bounds(hwnd: HWND) -> Option<PhysicalRect> {
    measure(hwnd)?
}

/// `None` if the query failed; `Some(None)` for a measured zero-area window.
fn measure(hwnd: HWND) -> Option<Option<PhysicalRect>> {
    let mut rect = RECT::default();
    // SAFETY: rect lives throughout the call; stale handles fail closed.
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    if rect.right == rect.left || rect.bottom == rect.top {
        return Some(None);
    }
    physical_rect(rect).map(Some)
}

unsafe extern "system" fn taskbars(hwnd: HWND, param: LPARAM) -> BOOL {
    if matches!(
        class(hwnd).as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    ) {
        // SAFETY: EnumWindows synchronously invokes us with this live Vec.
        unsafe { &mut *(param.0 as *mut Vec<HWND>) }.push(hwnd);
    }
    BOOL(1)
}

/// Cheap top-level window inventory; no child enumeration or UI Automation.
pub(crate) fn taskbar_handles() -> windows::core::Result<Vec<HWND>> {
    let mut handles = Vec::<HWND>::new();
    // SAFETY: synchronous callback/vector ownership; class strings are static.
    // Direct lookup supplements EnumWindows, which can omit modern shell roots
    // in hosted Windows 11/emulation environments. Neither path guesses bounds.
    unsafe {
        EnumWindows(
            Some(taskbars),
            LPARAM((&mut handles as *mut Vec<HWND>) as isize),
        )?;
        for name in [w!("Shell_TrayWnd"), w!("Shell_SecondaryTrayWnd")] {
            let mut after = None;
            for count in 0..=128 {
                let Ok(hwnd) = FindWindowExW(None, after, name, None) else {
                    break;
                };
                if count == 128 {
                    return Err(windows::core::Error::new(
                        windows::core::HRESULT(0x80004005u32 as i32),
                        "Native taskbar enumeration limit reached",
                    ));
                }
                if !handles.contains(&hwnd) {
                    handles.push(hwnd);
                }
                after = Some(hwnd);
            }
        }
    }
    // HWNDs are internal only; ordinal IDs remain generation-scoped. Changes
    // in unrelated window z-order must not invalidate an otherwise stable set.
    handles.sort_by_key(|hwnd| hwnd.0 as usize);
    Ok(handles)
}

unsafe extern "system" fn children(hwnd: HWND, param: LPARAM) -> BOOL {
    // SAFETY: EnumChildWindows synchronously invokes us with this live Vec.
    let records = unsafe { &mut *(param.0 as *mut Vec<NativeChild>) };
    // Bound the inventory; saturation means exclusions cannot be proven.
    if records.len() >= 2048 {
        return BOOL(0);
    }
    let measured = measure(hwnd);
    records.push(NativeChild {
        class: class(hwnd),
        rect: measured.flatten(),
        measured: measured.is_some(),
        // SAFETY: IsWindowVisible accepts a potentially stale window handle.
        visible: unsafe { IsWindowVisible(hwnd) }.as_bool(),
    });
    BOOL(1)
}

pub(crate) fn shell_identity(hwnd: HWND) -> windows::core::Result<(u32, u64)> {
    let mut pid = 0;
    // SAFETY: output pointers are live; owned process handle is always closed.
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?;
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let result = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
        let closed = CloseHandle(process);
        result?;
        closed?;
        Ok((
            pid,
            (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
        ))
    }
}

type NativeEpoch = (
    HWND,
    (u32, u64),
    Option<PhysicalRect>,
    PhysicalRect,
    u32,
    bool,
);

fn native_epoch(hwnd: HWND) -> windows::core::Result<NativeEpoch> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: read-only queries on this helper thread with a live output buffer.
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return Err(probe_error(
                "native epoch: GetMonitorInfoW",
                windows::core::Error::from_win32(),
            ));
        }
        let monitor_rect =
            physical_rect(info.rcMonitor).ok_or_else(windows::core::Error::from_win32)?;
        Ok((
            hwnd,
            shell_identity(hwnd)?,
            bounds(hwnd),
            monitor_rect,
            GetDpiForWindow(hwnd),
            IsWindowVisible(hwnd).as_bool(),
        ))
    }
}

fn probe_error(operation: &str, error: windows::core::Error) -> windows::core::Error {
    windows::core::Error::new(
        windows::core::HRESULT(0x80004005u32 as i32),
        format!("{operation}: {error}"),
    )
}

/// Exact build and update revision, without relying on version virtualization.
pub fn os_build() -> windows::core::Result<(String, u32)> {
    let mut buffer = [0u16; 64];
    let mut bytes = size_of::<[u16; 64]>() as u32;
    let mut revision = 0u32;
    let mut revision_bytes = size_of::<u32>() as u32;
    // SAFETY: fixed buffers and matching byte lengths; reads only, no registry edits.
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            w!("CurrentBuildNumber"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
        .ok()?;
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            w!("UBR"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut revision as *mut u32).cast()),
            Some(&mut revision_bytes),
        )
        .ok()?;
    }
    let end = buffer.iter().position(|v| *v == 0).unwrap_or(buffer.len());
    Ok((String::from_utf16_lossy(&buffer[..end]), revision))
}

/// Identity of the process that owns the primary taskbar, if one exists.
pub fn current_shell() -> Option<(u32, u64)> {
    // SAFETY: class-name lookup of a top-level window; failure means no shell.
    let hwnd = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) }.ok()?;
    shell_identity(hwnd).ok()
}

/// Returns value records only. Any shell replacement during a taskbar probe
/// invalidates the complete result. IDs are local ordinals, never HWND values.
pub fn collect(generation: ShellGeneration) -> windows::core::Result<Vec<TaskbarObservation>> {
    collect_with(generation, true)
}

/// `with_uia = false` skips the blocking UI Automation inventory entirely and
/// gives native queries only (the fixture and native-only helper mode).
pub fn collect_with(
    generation: ShellGeneration,
    with_uia: bool,
) -> windows::core::Result<Vec<TaskbarObservation>> {
    let handles = taskbar_handles().map_err(|e| probe_error("initial taskbar enumeration", e))?;
    let initial: Vec<_> = handles
        .iter()
        .map(|hwnd| native_epoch(*hwnd))
        .collect::<windows::core::Result<_>>()
        .map_err(|e| probe_error("initial native epoch", e))?;
    let mut observations = Vec::new();
    let mut monitors = Vec::new();
    for (index, hwnd) in handles.into_iter().enumerate() {
        let started = Instant::now();
        let identity =
            shell_identity(hwnd).map_err(|e| probe_error("taskbar shell identity", e))?;
        let taskbar_rect = bounds(hwnd).ok_or_else(|| {
            probe_error(
                "taskbar bounds unavailable",
                windows::core::Error::from_win32(),
            )
        })?;
        // SAFETY: monitor handle used only on this thread; info has the correct size.
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return Err(probe_error(
                "taskbar GetMonitorInfoW",
                windows::core::Error::from_win32(),
            ));
        }
        let monitor_rect =
            physical_rect(info.rcMonitor).ok_or_else(windows::core::Error::from_win32)?;
        let monitor_index = if let Some(i) = monitors.iter().position(|m| *m == monitor) {
            i
        } else {
            monitors.push(monitor);
            monitors.len() - 1
        };
        let mut native_children = Vec::<NativeChild>::new();
        // SAFETY: callback borrows the vector synchronously. No HWND is published.
        unsafe {
            let _ = EnumChildWindows(
                Some(hwnd),
                Some(children),
                LPARAM((&mut native_children as *mut Vec<NativeChild>) as isize),
            );
        }
        let uia = if with_uia {
            crate::taskbar::uia::inventory(hwnd)
        } else {
            crate::taskbar::uia::Inventory::default()
        };
        if shell_identity(hwnd)? != identity || bounds(hwnd) != Some(taskbar_rect) {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x80004005u32 as i32),
                "Shell identity or taskbar geometry changed during discovery",
            ));
        }
        observations.push(TaskbarObservation {
            hwnd: hwnd.0 as usize,
            snapshot: TaskbarSnapshot {
                key: TaskbarKey {
                    generation,
                    id: TaskbarId(index as u64 + 1),
                },
                taskbar_rect,
                clock_rect: None,
                monitor: MonitorId(monitor_index as u64 + 1),
                monitor_rect,
                // SAFETY: read-only query; zero remains observable, never guessed.
                dpi: unsafe { GetDpiForWindow(hwnd) },
                primary: info.dwFlags & 1 != 0,
                visibility: if unsafe { IsWindowVisible(hwnd) }.as_bool() {
                    TaskbarVisibility::Visible
                } else {
                    TaskbarVisibility::Hidden
                },
                strategy: DiscoveryStrategy::NativeChildren,
            },
            shell_pid: identity.0,
            shell_started: identity.1,
            native_children,
            native_inventory_incomplete: false,
            uia,
            elapsed_micros: started.elapsed().as_micros(),
        });
    }
    for observation in &mut observations {
        observation.native_inventory_incomplete = observation.native_children.len() >= 2048
            || observation.native_children.iter().any(|c| !c.measured);
    }
    let final_handles =
        taskbar_handles().map_err(|e| probe_error("final taskbar enumeration", e))?;
    let final_state: Vec<_> = final_handles
        .iter()
        .map(|hwnd| native_epoch(*hwnd))
        .collect::<windows::core::Result<_>>()
        .map_err(|e| probe_error("final native epoch", e))?;
    if initial != final_state {
        return Err(windows::core::Error::new(
            windows::core::HRESULT(0x80004005u32 as i32),
            "Taskbar set or shell identity changed during discovery",
        ));
    }
    Ok(observations)
}
