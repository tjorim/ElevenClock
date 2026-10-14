use std::fmt;
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

/// Native failures are retained for developer investigation, with a GUI error
/// surface for users. No stdout, console attachment or implicit fallback.
#[derive(Debug)]
pub struct PlatformError(windows::core::Error);
impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Could not open ElevenClock Settings: {}", self.0)
    }
}
impl std::error::Error for PlatformError {}
impl From<windows::core::Error> for PlatformError {
    fn from(value: windows::core::Error) -> Self {
        Self(value)
    }
}
impl PlatformError {
    pub fn show(&self) {
        let text: Vec<u16> = self.to_string().encode_utf16().chain(Some(0)).collect();
        // SAFETY: text is NUL terminated and lives throughout this synchronous call.
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(text.as_ptr()),
                w!("ElevenClock"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

/// Runs the modal prototype on the calling UI thread until Close/Escape/X.
/// The dialog manager owns controls, fonts and DPI-v2 layout. No HWND escapes
/// this call and no worker threads or application state are installed yet.
pub fn run_settings() -> Result<(), PlatformError> {
    // SAFETY: null module name resolves this executable. The initialized stack
    // structure lives for the call; all resources are compiled into the app.
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
        let result = DialogBoxParamW(
            Some(module.into()),
            PCWSTR(101_usize as *const u16),
            None,
            Some(dialog_proc),
            LPARAM(icon.0 as isize),
        );
        if result == -1 {
            return Err(windows::core::Error::from_win32().into());
        }
        Ok(())
    }
}

// SAFETY contract: called synchronously by the dialog manager for live windows.
// Never unwind through this callback. No allocations with fallible application
// logic, blocking work, borrowed callback state or stored HWNDs are involved.
unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match message {
        WM_INITDIALOG => {
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
                        1001,
                        CB_ADDSTRING,
                        WPARAM(0),
                        LPARAM(label.as_ptr() as isize),
                    );
                }
                SendDlgItemMessageW(hwnd, 1001, CB_SETCURSEL, WPARAM(0), LPARAM(0));
            }
            1 // Dialog manager sets focus to first WS_TABSTOP control.
        }
        WM_COMMAND if (wparam.0 & 0xffff) == IDCANCEL.0 as usize => {
            // SAFETY: ends this modal dialog; manager destroys its child windows.
            unsafe {
                let _ = EndDialog(hwnd, 0);
            }
            1
        }
        WM_COMMAND if (wparam.0 & 0xffff) == 1001 && (wparam.0 >> 16) == CBN_SELCHANGE as usize => {
            // Standard controls follow Windows accessibility colors. Documented
            // Win32 APIs do not provide universal dark styling for this dialog.
            // The prototype explicitly reports the limitation, not a fake theme.
            unsafe {
                let selection = SendDlgItemMessageW(hwnd, 1001, CB_GETCURSEL, WPARAM(0), LPARAM(0));
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
            // SAFETY: same lifetime rules as Close/Escape.
            unsafe {
                let _ = EndDialog(hwnd, 0);
            }
            1
        }
        _ => 0, // Default dialog handling includes keyboard and DPI-v2 scaling.
    }
}
