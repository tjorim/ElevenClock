use std::{cell::Cell, fmt};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx},
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, w},
};

/// Resource ids shared with `settings.rc` and the hosted UI checks.
pub const ID_APPEARANCE: i32 = 1001;
pub const ID_EXIT: i32 = 1003;

/// Native failures are retained for developer investigation, with a GUI error
/// surface for users. No stdout, console attachment or implicit fallback.
#[derive(Debug)]
pub struct PlatformError {
    context: &'static str,
    source: windows::core::Error,
}
impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.context, self.source)
    }
}
impl std::error::Error for PlatformError {}
impl From<windows::core::Error> for PlatformError {
    fn from(source: windows::core::Error) -> Self {
        Self::new("ElevenClock-rs could not start", source)
    }
}
impl PlatformError {
    pub fn new(context: &'static str, source: windows::core::Error) -> Self {
        Self { context, source }
    }
    /// An error with no underlying Win32 failure, such as an unresponsive peer.
    pub fn message(context: &'static str) -> Self {
        Self::new(
            context,
            windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), "Failed"),
        )
    }
    pub fn show(&self) {
        let text: Vec<u16> = self.to_string().encode_utf16().chain(Some(0)).collect();
        // SAFETY: text is NUL terminated and lives throughout this synchronous call.
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(text.as_ptr()),
                w!("ElevenClock-rs"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

thread_local! {
    /// The one open Settings window of this (UI) thread, or null.
    static OPEN: Cell<isize> = const { Cell::new(0) };
}

/// The Settings shell. It is modeless: closing it only destroys the window, so the
/// clock keeps running in the background. `Exit` asks the host to quit instead.
/// Everything runs on the UI thread; the dialog manager owns controls, fonts and
/// DPI-v2 layout, and no HWND escapes this type.
pub struct SettingsWindow {
    module: windows::Win32::Foundation::HINSTANCE,
    icon: isize,
}

impl SettingsWindow {
    pub fn new() -> Result<Self, PlatformError> {
        // SAFETY: null module name resolves this executable; the initialized
        // stack structure lives for the call; resources are compiled into the app.
        unsafe {
            let module = GetModuleHandleW(None)?;
            let icon = LoadIconW(Some(module.into()), PCWSTR(102_usize as *const u16))?;
            let controls = INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_STANDARD_CLASSES,
            };
            if !InitCommonControlsEx(&controls).as_bool() {
                return Err(windows::core::Error::from_win32().into());
            }
            Ok(Self {
                module: module.into(),
                icon: icon.0 as isize,
            })
        }
    }

    pub fn is_open(&self) -> bool {
        OPEN.get() != 0
    }

    /// Open the window, or restore and focus the existing one. Never creates a
    /// second window.
    pub fn show_or_focus(&self) -> Result<(), PlatformError> {
        // SAFETY: the stored handle was published by this thread's dialog and is
        // cleared in WM_NCDESTROY; creation parameters are plain integers.
        unsafe {
            let existing = HWND(OPEN.get() as *mut _);
            if OPEN.get() != 0 && IsWindow(Some(existing)).as_bool() {
                if IsIconic(existing).as_bool() {
                    let _ = ShowWindow(existing, SW_RESTORE);
                }
                let _ = SetForegroundWindow(existing);
                return Ok(());
            }
            let hwnd = CreateDialogParamW(
                Some(self.module),
                PCWSTR(101_usize as *const u16),
                None,
                Some(dialog_proc),
                LPARAM(self.icon),
            )?;
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
        Ok(())
    }

    /// Give the dialog manager first refusal on keyboard messages (Tab, arrows,
    /// mnemonics, Enter, Escape). Returns whether the message was consumed.
    pub fn handle_message(&self, message: &mut MSG) -> bool {
        let hwnd = OPEN.get();
        // SAFETY: the dialog handle is live while OPEN is non-null.
        hwnd != 0 && unsafe { IsDialogMessageW(HWND(hwnd as *mut _), message) }.as_bool()
    }

    pub fn close(&self) {
        let hwnd = OPEN.get();
        if hwnd != 0 {
            // SAFETY: destroys our own dialog on its owning thread.
            let _ = unsafe { DestroyWindow(HWND(hwnd as *mut _)) };
        }
    }
}

// SAFETY contract: called synchronously by the dialog manager for live windows
// on the UI thread. Never unwind through this callback; nothing here allocates
// fallibly, blocks, or stores borrowed state.
unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match message {
        WM_INITDIALOG => {
            OPEN.set(hwnd.0 as isize);
            // SAFETY: all control IDs and resources belong to the live dialog;
            // shared icons returned by LoadIconW must NOT be destroyed.
            unsafe {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_BIG as usize)),
                    Some(lparam),
                );
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_SMALL as usize)),
                    Some(lparam),
                );
                for label in [w!("System"), w!("Light"), w!("Dark")] {
                    SendDlgItemMessageW(
                        hwnd,
                        ID_APPEARANCE,
                        CB_ADDSTRING,
                        WPARAM(0),
                        LPARAM(label.as_ptr() as isize),
                    );
                }
                SendDlgItemMessageW(hwnd, ID_APPEARANCE, CB_SETCURSEL, WPARAM(0), LPARAM(0));
            }
            1 // Dialog manager sets focus to first WS_TABSTOP control.
        }
        // Close, Escape and the system menu close only this window. The clock
        // keeps running; the host never learns of the close.
        WM_COMMAND if (wparam.0 & 0xffff) == IDCANCEL.0 as usize => {
            // SAFETY: destroys the live dialog from its own procedure.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            1
        }
        WM_COMMAND if (wparam.0 & 0xffff) == ID_EXIT as usize => {
            crate::runtime::native::request_exit();
            // SAFETY: as above.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            1
        }
        WM_COMMAND
            if (wparam.0 & 0xffff) == ID_APPEARANCE as usize
                && (wparam.0 >> 16) == CBN_SELCHANGE as usize =>
        {
            // Standard controls follow Windows accessibility colors. Documented
            // Win32 APIs do not provide universal dark styling for this dialog.
            // Preferences are not yet editable here; R10 owns the full form.
            unsafe {
                let selection =
                    SendDlgItemMessageW(hwnd, ID_APPEARANCE, CB_GETCURSEL, WPARAM(0), LPARAM(0));
                let message = if selection.0 == 2 {
                    w!(
                        "Dark appearance is unavailable in this preview. Windows colors remain active."
                    )
                } else {
                    w!(
                        "Standard controls use Windows accessibility colors. Preferences are not saved."
                    )
                };
                let _ = SetDlgItemTextW(hwnd, 1002, message);
            }
            1
        }
        WM_CLOSE => {
            // SAFETY: as above.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            1
        }
        WM_NCDESTROY => {
            if OPEN.get() == hwnd.0 as isize {
                OPEN.set(0);
            }
            0
        }
        _ => 0, // Default dialog handling includes keyboard and DPI-v2 scaling.
    }
}
