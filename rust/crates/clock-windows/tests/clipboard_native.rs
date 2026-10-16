//! The real Windows clipboard (R11). These tests overwrite the clipboard text while
//! they run (they put the previous text back), so they are ignored by default:
//!
//! ```text
//! cargo test -p clock-windows --test clipboard_native -- --ignored --test-threads=1
//! ```
//!
//! CI runs exactly that on its Windows runner.
#![cfg(windows)]

use clock_windows::support::{Clipboard, ClipboardFault, WindowsClipboard};
use std::{sync::mpsc, thread, time::Duration};
use windows::{
    Win32::{
        Foundation::HWND,
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard},
            Memory::{GlobalLock, GlobalUnlock},
            Ole::CF_UNICODETEXT,
        },
        UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP},
    },
    core::w,
};

/// A hidden window of the calling thread: the clipboard owner.
struct Owner(HWND);
impl Owner {
    fn new() -> Self {
        // SAFETY: a predefined class creates a plain window on this thread.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("clipboard test owner"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
        }
        .expect("owner window");
        Self(hwnd)
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        // SAFETY: destroys the window created in `new` on its own thread.
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

/// Current clipboard text, or None if there is none (or it stays busy).
fn read(owner: &Owner) -> Option<String> {
    for _ in 0..50 {
        // SAFETY: balanced Open/Close; the locked block is only read while locked.
        unsafe {
            if OpenClipboard(Some(owner.0)).is_ok() {
                let text = GetClipboardData(u32::from(CF_UNICODETEXT.0))
                    .ok()
                    .and_then(|handle| {
                        let global = windows::Win32::Foundation::HGLOBAL(handle.0);
                        let pointer = GlobalLock(global).cast::<u16>();
                        if pointer.is_null() {
                            return None;
                        }
                        let mut length = 0;
                        while *pointer.add(length) != 0 {
                            length += 1;
                        }
                        let text =
                            String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length));
                        let _ = GlobalUnlock(global);
                        Some(text)
                    });
                let _ = CloseClipboard();
                return text;
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    None
}

fn restore(owner: &Owner, previous: Option<String>) {
    match previous {
        Some(text) => {
            let _ = WindowsClipboard::new(owner.0).set_text(&text);
        }
        None => {
            // SAFETY: balanced Open/Close.
            unsafe {
                if OpenClipboard(Some(owner.0)).is_ok() {
                    let _ = EmptyClipboard();
                    let _ = CloseClipboard();
                }
            }
        }
    }
}

#[test]
#[ignore = "overwrites the clipboard text while it runs"]
fn copies_multiline_unicode_text_exactly() {
    let owner = Owner::new();
    let previous = read(&owner);
    let report = "ElevenClock-rs diagnostics\r\nMonitor: 1920x1080 px at (-1920, 0)\r\nCafé ✓\r\n";
    let result = WindowsClipboard::new(owner.0).set_text(report);
    let copied = read(&owner);
    restore(&owner, previous);
    assert_eq!(result, Ok(()));
    assert_eq!(copied.as_deref(), Some(report));
}

#[test]
#[ignore = "overwrites the clipboard text while it runs"]
fn a_busy_clipboard_fails_quickly_and_leaves_the_old_contents() {
    let owner = Owner::new();
    let previous = read(&owner);
    assert_eq!(
        WindowsClipboard::new(owner.0).set_text("the old contents"),
        Ok(())
    );

    // Another program holds the clipboard open for far longer than we will wait.
    let (opened, wait_opened) = mpsc::channel();
    let (release, wait_release) = mpsc::channel::<()>();
    let holder = thread::spawn(move || {
        let other = Owner::new();
        // SAFETY: balanced Open/Close on the holder's own thread.
        unsafe {
            OpenClipboard(Some(other.0)).expect("holder opens the clipboard");
            opened.send(()).unwrap();
            let _ = wait_release.recv();
            let _ = CloseClipboard();
        }
    });
    wait_opened.recv().unwrap();
    let started = std::time::Instant::now();
    let result = WindowsClipboard::new(owner.0).set_text("the new report");
    let waited = started.elapsed();
    release.send(()).unwrap();
    holder.join().unwrap();

    let after = read(&owner);
    restore(&owner, previous);
    let error = result.expect_err("a held clipboard cannot be written");
    assert_eq!(error.fault, ClipboardFault::Busy);
    assert!(!error.replaced, "the clipboard must not have been cleared");
    assert!(
        waited < Duration::from_millis(500),
        "UI thread blocked {waited:?}"
    );
    assert_eq!(after.as_deref(), Some("the old contents"));
}
