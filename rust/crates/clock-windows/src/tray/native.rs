//! `Shell_NotifyIconW` registration (Windows, UI thread only).
use super::WM_TRAYICON;
use crate::interaction::tray_tooltip;
use windows::Win32::{
    Foundation::HWND,
    UI::{
        Shell::{
            NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
            NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
        },
        WindowsAndMessaging::HICON,
    },
};

/// Identifies this app's single icon among those owned by the host window.
const ICON_ID: u32 = 1;

/// The notification-area icon. Dropping it removes the icon, so every exit path
/// that drops the platform leaves nothing behind in the tray.
pub struct Tray {
    host: HWND,
    icon: HICON,
    paused: bool,
    registered: bool,
}

impl Tray {
    /// `icon` must stay valid for the life of the tray (a shared resource icon).
    pub fn new(host: HWND, icon: HICON) -> Self {
        Self {
            host,
            icon,
            paused: false,
            registered: false,
        }
    }

    pub fn is_registered(&self) -> bool {
        self.registered
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.host,
            uID: ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAYICON,
            hIcon: self.icon,
            ..Default::default()
        };
        // szTip is 128 UTF-16 units including the terminator; the tooltip is short.
        for (slot, unit) in data
            .szTip
            .iter_mut()
            .zip(tray_tooltip(self.paused).encode_utf16())
        {
            *slot = unit;
        }
        data
    }

    /// Add the icon, or re-add it after Explorer recreated the notification area.
    /// Returns whether the icon is now registered; failure is expected while
    /// Explorer is still starting, and the next `TaskbarCreated` tries again.
    pub fn restore(&mut self) -> bool {
        let mut data = self.data();
        // SAFETY: `data` is a fully initialized structure of the advertised size.
        unsafe {
            // After an Explorer restart the old entry is gone and ADD succeeds. If
            // it still exists (a repeated restore request), ADD fails and MODIFY
            // refreshes it instead of stacking a second icon.
            let added = Shell_NotifyIconW(NIM_ADD, &data).as_bool()
                || Shell_NotifyIconW(NIM_MODIFY, &data).as_bool();
            if added {
                // Version 4 delivers keyboard selection and the anchor point.
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
            }
            self.registered = added;
        }
        self.registered
    }

    /// Update the tooltip for the replacement mode.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if self.registered {
            let data = self.data();
            // SAFETY: as above.
            self.registered = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool();
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.host,
            uID: ICON_ID,
            ..Default::default()
        };
        // SAFETY: deleting this icon by (window, id); a missing icon is harmless.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }
}
