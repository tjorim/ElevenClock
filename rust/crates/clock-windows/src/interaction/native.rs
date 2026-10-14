//! Windows side of the clock actions (R09): thread-safe interaction hand-off,
//! the deliberate calendar keystroke batch and the fixed popup menu.
use super::{
    CalendarOutcome, HeldKeys, Interaction, InteractionQueue, Key, KeyEvent, MenuCommand,
    ScreenPoint, WIN_N, cleanup_after, menu_items,
};
use std::sync::{
    Mutex,
    atomic::{AtomicIsize, Ordering},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, POINT, WPARAM},
        UI::{
            Input::KeyboardAndMouse::*,
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, MF_STRING, PostMessageW,
                SetForegroundWindow, SetMenuDefaultItem, TPM_BOTTOMALIGN, TPM_RETURNCMD,
                TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_APP, WM_NULL,
            },
        },
    },
    core::HSTRING,
};

/// Posted to the host window when an interaction is queued from any thread.
pub const WM_INTERACTION: u32 = WM_APP + 4;

/// Shared with the UI Automation provider, whose methods run on a thread of
/// UIA's choosing: the queue is the only state that crosses threads.
static QUEUE: Mutex<Option<InteractionQueue>> = Mutex::new(None);
static HOST: AtomicIsize = AtomicIsize::new(0);

pub fn set_host(host: HWND) {
    HOST.store(host.0 as isize, Ordering::Release);
}
pub fn clear_host() {
    HOST.store(0, Ordering::Release);
}

/// Queue an interaction and wake the loop. Non-blocking apart from the brief
/// queue lock; safe from window procedures and from the UIA provider thread.
pub(crate) fn post_interaction(interaction: Interaction) {
    {
        let mut queue = QUEUE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.get_or_insert_with(Default::default).push(interaction);
    }
    let host = HOST.load(Ordering::Acquire);
    if host != 0 {
        // SAFETY: the host window outlives every overlay; a failed post (window
        // already gone) only means nobody is left to act, and the next message wakes
        // the loop anyway.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(host as *mut _)),
                WM_INTERACTION,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

pub(crate) fn take_interactions() -> Vec<Interaction> {
    QUEUE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_mut()
        .map(InteractionQueue::take)
        .unwrap_or_default()
}

fn down(key: VIRTUAL_KEY) -> bool {
    // SAFETY: a stateless keyboard query. The high bit means "currently down".
    unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 }
}
pub(crate) fn held_keys() -> HeldKeys {
    HeldKeys {
        win: down(VK_LWIN) || down(VK_RWIN),
        ctrl: down(VK_CONTROL),
        shift: down(VK_SHIFT),
        alt: down(VK_MENU),
        n: down(VIRTUAL_KEY(0x4e)),
    }
}

fn input(event: KeyEvent) -> INPUT {
    let key = match event.key {
        Key::Win => VK_LWIN,
        Key::N => VIRTUAL_KEY(0x4e),
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                dwFlags: if event.up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                ..Default::default()
            },
        },
    }
}

/// One deliberate Win+N. Sent as a single batch so other input cannot interleave;
/// never retried; refused while the user holds a modifier or `N`.
pub(crate) fn send_win_n() -> CalendarOutcome {
    if held_keys().blocks_calendar() {
        return CalendarOutcome::ModifiersHeld;
    }
    let batch = WIN_N.map(input);
    // SAFETY: the slice and element size describe live, initialized INPUT values.
    let inserted = unsafe { SendInput(&batch, std::mem::size_of::<INPUT>() as i32) } as usize;
    if inserted == batch.len() {
        return CalendarOutcome::Sent;
    }
    let error = windows::core::Error::from_win32();
    let cleanup: Vec<INPUT> = cleanup_after(inserted).into_iter().map(input).collect();
    if !cleanup.is_empty() {
        // SAFETY: as above. Releases only keys whose injected key-down was accepted.
        unsafe {
            SendInput(&cleanup, std::mem::size_of::<INPUT>() as i32);
        }
    }
    CalendarOutcome::Failed(format!(
        "Windows accepted {inserted} of {} keystrokes: {error}",
        batch.len()
    ))
}

/// Show the fixed menu and return the chosen command, if any. Modal: it returns
/// when the menu closes. The menu may take focus; the overlay never does.
pub(crate) fn show_menu(
    host: HWND,
    anchor: Option<ScreenPoint>,
    paused: bool,
) -> Option<MenuCommand> {
    // SAFETY: creates, uses and destroys one menu on this thread; every string
    // outlives its call; the host window is ours.
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let mut built = true;
        for item in menu_items(paused) {
            built &= AppendMenuW(
                menu,
                MF_STRING,
                item.command.id() as usize,
                &HSTRING::from(item.label),
            )
            .is_ok();
        }
        if !built {
            let _ = DestroyMenu(menu);
            return None;
        }
        let _ = SetMenuDefaultItem(menu, MenuCommand::Settings.id(), 0);
        let point = anchor.map_or_else(
            || {
                let mut at = POINT::default();
                let _ = GetCursorPos(&mut at);
                at
            },
            |p| POINT { x: p.x, y: p.y },
        );
        // Documented requirement for menus owned by a non-foreground window:
        // without it the menu would not close when the user clicks elsewhere.
        let _ = SetForegroundWindow(host);
        let chosen = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0,
            point.x,
            point.y,
            host,
            None,
        );
        // Documented follow-up that lets the previous foreground window recover.
        let _ = PostMessageW(Some(host), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        MenuCommand::from_id(chosen.0 as u32)
    }
}
