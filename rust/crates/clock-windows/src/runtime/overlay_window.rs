//! One overlay HWND and its drawing surface (Windows, UI thread only).
//!
//! The window is a borderless, non-activating tool window sized to the verified
//! clock rectangle. It is never parented into Explorer, never changes Explorer's
//! styles and never uses an unconditional global topmost: z-order mirrors the
//! visible taskbar's own band, immediately above it (R03 strategy, unchanged).
use crate::{
    overlay::{BandAction, band_action},
    render::{ClockSurface, PaintOutcome, PaintRequest, RenderError, Renderer},
};
use clock_core::PhysicalRect;
use std::sync::atomic::{AtomicU64, Ordering};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::ValidateRect,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
    core::{Result, w},
};

/// Bumped by the window procedure (UI thread) on messages that invalidate pixels;
/// the loop compares it and repaints from stored state. The callback keeps no
/// references to loop state.
pub static REPAINT_EPOCH: AtomicU64 = AtomicU64::new(0);
const CLASS_NAME: windows::core::PCWSTR = w!("ElevenClock.Overlay");

/// Registers the window class for the life of the runtime.
pub struct OverlayClass(windows::Win32::Foundation::HINSTANCE);
impl OverlayClass {
    pub fn register() -> Result<Self> {
        // SAFETY: standard class registration with a static procedure/name.
        unsafe {
            let module = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: module.into(),
                lpszClassName: CLASS_NAME,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(windows::core::Error::from_win32());
            }
            Ok(Self(module.into()))
        }
    }
}
impl Drop for OverlayClass {
    fn drop(&mut self) {
        // SAFETY: all overlay windows are destroyed before the class guard drops.
        let _ = unsafe { UnregisterClassW(CLASS_NAME, Some(self.0)) };
    }
}

pub struct Overlay {
    // Declared first: the surface releases its Direct2D target before the HWND dies.
    pub surface: ClockSurface,
    hwnd: HWND,
    taskbar: HWND,
    bounds: PhysicalRect,
    pub last_text: Option<clock_core::ClockText>,
    shown: bool,
}
impl Drop for Overlay {
    fn drop(&mut self) {
        // SAFETY: destroying a window created and owned by this thread.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn topmost(hwnd: HWND) -> bool {
    // SAFETY: style query on a possibly stale handle; failure reads as zero.
    unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0 }
}

/// Nearest predecessor of the taskbar in its own z-order band. Windows can list
/// windows of other bands between taskbar entries; anchoring after one of those
/// would demote the overlay.
fn relative_predecessor(root: HWND, root_topmost: bool) -> Result<Option<HWND>> {
    let mut cursor = root;
    for _ in 0..128 {
        // SAFETY: z-order walk over handles that may go stale; errors end the walk.
        let Ok(previous) = (unsafe { GetWindow(cursor, GW_HWNDPREV) }) else {
            return Ok(None);
        };
        if previous.is_invalid() {
            return Ok(None);
        }
        if topmost(previous) == root_topmost {
            return Ok(Some(previous));
        }
        cursor = previous;
    }
    Err(windows::core::Error::new(
        windows::core::HRESULT(0x80004005u32 as i32),
        "Taskbar predecessor inventory exceeded its bound",
    ))
}

impl Overlay {
    pub fn create(renderer: &Renderer, taskbar: HWND, bounds: PhysicalRect) -> Result<Self> {
        // SAFETY: class registered by OverlayClass; creates a hidden popup owned
        // by this thread. WS_EX_NOACTIVATE/TOOLWINDOW keep it out of Alt-Tab and
        // focus. It is never shown here.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!("ElevenClock"),
                WS_POPUP,
                bounds.left(),
                bounds.top(),
                bounds.width(),
                bounds.height(),
                None,
                None,
                Some(GetModuleHandleW(None)?.into()),
                None,
            )?
        };
        Ok(Self {
            surface: renderer.create_surface(hwnd),
            hwnd,
            taskbar,
            bounds,
            last_text: None,
            shown: false,
        })
    }

    pub fn set_bounds(&mut self, bounds: PhysicalRect) -> Result<()> {
        self.bounds = bounds;
        // SAFETY: moves our own window without activation or z-order change.
        unsafe {
            SetWindowPos(
                self.hwnd,
                None,
                bounds.left(),
                bounds.top(),
                bounds.width(),
                bounds.height(),
                SWP_NOACTIVATE | SWP_NOZORDER,
            )
        }
    }

    pub fn paint(
        &mut self,
        renderer: &Renderer,
        request: &PaintRequest<'_>,
    ) -> std::result::Result<PaintOutcome, RenderError> {
        self.surface.paint(renderer, request)
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    pub fn hide(&mut self) {
        // SAFETY: hiding our own window; also demote so a hidden overlay never
        // lingers in the taskbar's topmost band.
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            if topmost(self.hwnd) {
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_NOTOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
        self.shown = false;
    }

    /// True when the overlay already sits immediately above its taskbar.
    pub fn is_anchored(&self) -> bool {
        if !self.shown {
            return false;
        }
        let root_topmost = topmost(self.taskbar);
        topmost(self.hwnd) == root_topmost
            && matches!(relative_predecessor(self.taskbar, root_topmost), Ok(Some(p)) if p == self.hwnd)
    }

    /// Show (or re-anchor) the overlay directly above its taskbar in the
    /// taskbar's band. Call only after a successful paint: nothing is exposed
    /// until its pixels are correct. Failure hides the overlay.
    pub fn show_anchored(&mut self) -> Result<()> {
        let root_topmost = topmost(self.taskbar);
        let band = band_action(true, root_topmost, topmost(self.hwnd));
        if band != BandAction::Preserve {
            // SAFETY: hide first so temporary promotion is never visible above
            // unrelated topmost windows; only our own HWND changes band.
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
                let target = if band == BandAction::PromoteToTaskbar {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                };
                SetWindowPos(
                    self.hwnd,
                    Some(target),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                )?;
            }
        }
        let preceding = relative_predecessor(self.taskbar, root_topmost)?.unwrap_or(HWND_TOP);
        let adjacent = preceding == self.hwnd;
        let insert = if preceding.is_invalid() || adjacent {
            HWND_TOP
        } else {
            preceding
        };
        let keep_order = if adjacent {
            SWP_NOZORDER
        } else {
            SET_WINDOW_POS_FLAGS(0)
        };
        let b = self.bounds;
        // SAFETY: positions/shows our own window without activation.
        let positioned = unsafe {
            SetWindowPos(
                self.hwnd,
                Some(insert),
                b.left(),
                b.top(),
                b.width(),
                b.height(),
                SWP_NOACTIVATE | SWP_SHOWWINDOW | keep_order,
            )
        };
        match positioned {
            Ok(()) => {
                self.shown = true;
                Ok(())
            }
            Err(error) => {
                self.hide();
                Err(error)
            }
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: called by the system for our own windows on the UI thread.
    unsafe {
        match message {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT | WM_DPICHANGED | WM_DISPLAYCHANGE => {
                // Drawing happens in the loop, which owns the surface; the
                // callback only records that the pixels were invalidated.
                REPAINT_EPOCH.fetch_add(1, Ordering::Relaxed);
                let _ = ValidateRect(Some(hwnd), None);
                LRESULT(0)
            }
            // Never forward a close request: the host owns this window's lifetime.
            WM_CLOSE => LRESULT(0),
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    }
}
