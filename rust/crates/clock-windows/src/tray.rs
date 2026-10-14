//! The app's notification-area icon (R09).
//!
//! The icon exists so the fixed menu stays reachable without a pointer: the
//! notification area is keyboard navigable (Win+B), and keyboard activation,
//! the context-menu key and mouse selection all open the same menu as a right
//! click on the clock. The icon is registered with `NOTIFYICON_VERSION_4`, whose
//! callback encodes the event in the low word of `lparam` and the anchor point in
//! `wparam`. That decoding is portable and tested here; registration is native.
//!
//! Explorer restart: the runtime already detects the broadcast `TaskbarCreated`
//! (and Explorer's process identity) and calls `Platform::restore_tray` from that
//! one recovery path. This module has no timer, poll or retry of its own.
use crate::interaction::{Interaction, ScreenPoint};

/// Private callback message for the icon, posted to the hidden host window.
pub const WM_TRAYICON: u32 = 0x8000 + 5;

/// Low-word events of the version 4 callback that open the menu.
const WM_CONTEXTMENU: u32 = 0x007B;
/// Mouse selection (left click released on the icon).
const NIN_SELECT: u32 = 0x0400;
/// Keyboard selection (Enter or Space on the focused icon).
const NIN_KEYSELECT: u32 = 0x0401;

/// Decode a version 4 tray callback. Hover, balloon and popup notifications and
/// mouse-move traffic produce no action.
pub fn interaction_for_message(wparam: usize, lparam: isize) -> Option<Interaction> {
    let event = (lparam as usize & 0xFFFF) as u32;
    matches!(event, WM_CONTEXTMENU | NIN_SELECT | NIN_KEYSELECT).then(|| {
        // GET_X_LPARAM/GET_Y_LPARAM: signed words, so monitors left of or above
        // the primary one anchor correctly.
        let x = i32::from((wparam & 0xFFFF) as u16 as i16);
        let y = i32::from(((wparam >> 16) & 0xFFFF) as u16 as i16);
        Interaction::Menu {
            anchor: Some(ScreenPoint { x, y }),
        }
    })
}

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::Tray;

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(x: i16, y: i16) -> usize {
        (x as u16 as usize) | ((y as u16 as usize) << 16)
    }

    #[test]
    fn mouse_keyboard_and_context_key_open_the_same_menu_at_the_anchor() {
        for event in [WM_CONTEXTMENU, NIN_SELECT, NIN_KEYSELECT] {
            assert_eq!(
                interaction_for_message(pack(900, 1040), event as isize),
                Some(Interaction::Menu {
                    anchor: Some(ScreenPoint { x: 900, y: 1040 })
                }),
                "{event:#x}"
            );
        }
    }

    #[test]
    fn negative_monitor_coordinates_survive_the_word_packing() {
        assert_eq!(
            interaction_for_message(pack(-1800, -20), WM_CONTEXTMENU as isize),
            Some(Interaction::Menu {
                anchor: Some(ScreenPoint { x: -1800, y: -20 })
            })
        );
    }

    #[test]
    fn the_high_word_of_lparam_is_the_icon_id_and_never_the_event() {
        let with_id = (7isize << 16) | NIN_KEYSELECT as isize;
        assert!(interaction_for_message(0, with_id).is_some());
        let only_id = 7isize << 16;
        assert_eq!(interaction_for_message(0, only_id), None);
    }

    #[test]
    fn other_notifications_do_nothing() {
        // WM_MOUSEMOVE, WM_LBUTTONDOWN, NIN_BALLOONSHOW, NIN_POPUPOPEN, NIN_POPUPCLOSE.
        for event in [0x0200, 0x0201, 0x0402, 0x0406, 0x0407, 0] {
            assert_eq!(interaction_for_message(0, event), None, "{event:#x}");
        }
    }
}

#[cfg(all(test, windows))]
mod native_values {
    use windows::Win32::UI::{Shell::NIN_SELECT, WindowsAndMessaging::WM_CONTEXTMENU};

    /// The portable decoder duplicates Win32 numbers so it can be tested anywhere;
    /// this keeps those copies honest.
    #[test]
    fn duplicated_values_match_the_windows_definitions() {
        assert_eq!(super::WM_CONTEXTMENU, WM_CONTEXTMENU);
        assert_eq!(super::NIN_SELECT, NIN_SELECT);
        // NIN_KEYSELECT is documented as NINF_KEY (1) | NIN_SELECT; the bindings omit it.
        assert_eq!(super::NIN_KEYSELECT, NIN_SELECT | 1);
    }
}
