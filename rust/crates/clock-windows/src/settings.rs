//! The native Settings window (R10).
//!
//! A standard Win32 dialog (`settings.rc`) with a tab strip and four page dialogs:
//! Date and time, Appearance, Startup and Support. The dialog manager owns fonts,
//! DPI-v2 layout, tab order and mnemonics. This file only reads and writes
//! controls: every rule (validation, preview, fit, Apply/Cancel/Reset, recovery)
//! lives in `settings_model`.
//!
//! Window procedures never touch application state. They queue a small
//! [`UiEvent`], and [`SettingsWindow::service`] handles the queue on the UI thread
//! after the message returns, with the runtime in hand. That keeps callbacks
//! re-entrancy free (Apply may show a message box and call back into the runtime)
//! and lets the same loop that serves the clock serve this window.
use crate::{
    preferences_store::{PreferenceSession, PreferencesStore},
    runtime::{Input, Platform, Reason, Runtime},
    settings_model::{ApplyOutcome, FONT_MAX, FONT_MIN, Field, SettingsController, TOKEN_HELP},
};
use clock_core::{Monitors, Theme, format::PRESETS};
use std::{cell::Cell, cell::RefCell, fmt};
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, POINT, RECT, WPARAM},
        Graphics::Gdi::ScreenToClient,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Accessibility::NotifyWinEvent,
            Controls::{
                EM_LIMITTEXT, EM_SETSEL, ICC_STANDARD_CLASSES, ICC_TAB_CLASSES, ICC_UPDOWN_CLASS,
                INITCOMMONCONTROLSEX, InitCommonControlsEx, NMHDR, TCIF_TEXT, TCITEMW,
                TCM_GETCURSEL, TCM_INSERTITEMW, TCM_SETCURSEL, TCN_SELCHANGE, UDM_SETBUDDY,
                UDM_SETRANGE32,
            },
            Input::KeyboardAndMouse::{
                GetFocus, GetKeyState, SetFocus, VK_CONTROL, VK_NEXT, VK_PRIOR, VK_SHIFT, VK_TAB,
            },
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, PWSTR, w},
};

// Resource ids shared with `settings.rc`, the hosted UI checks and the tests.
const DIALOG: usize = 101;
const PAGE_DATE: i32 = 111;
const PAGE_APPEARANCE: i32 = 112;
const PAGE_STARTUP: i32 = 113;
const PAGE_SUPPORT: i32 = 114;
const PAGES: [i32; 4] = [PAGE_DATE, PAGE_APPEARANCE, PAGE_STARTUP, PAGE_SUPPORT];
const PAGE_TITLES: [&str; 4] = ["Date and time", "Appearance", "Startup", "Support"];

pub const ID_TAB: i32 = 1100;
const ID_INTRO: i32 = 1101;
const ID_STATUS: i32 = 1102;
pub const ID_EXIT: i32 = 1003;
pub const ID_RESET: i32 = 1104;
pub const ID_APPLY: i32 = 1105;
const ID_ANCHOR: i32 = 1106;

pub const ID_PRESET: i32 = 1201;
pub const ID_FORMAT: i32 = 1202;
const ID_FORMAT_ERROR: i32 = 1203;
pub const ID_PREVIEW: i32 = 1204;
const ID_TOKENS: i32 = 1205;

pub const ID_MONITORS_PRIMARY: i32 = 1301;
pub const ID_MONITORS_ALL: i32 = 1302;
pub const ID_FONT: i32 = 1303;
const ID_FONT_SPIN: i32 = 1304;
const ID_FONT_ERROR: i32 = 1305;
pub const ID_THEME: i32 = 1306;

const ID_STARTUP_INFO: i32 = 1401;
const ID_SUPPORT_INFO: i32 = 1501;

const EN_CHANGE: u32 = 0x0300;

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

/// What a window procedure saw. Values only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UiEvent {
    Command { id: i32, code: u32 },
    PageChanged,
    Relayout,
    Close,
}

thread_local! {
    /// The one open Settings window of this (UI) thread, or null.
    static OPEN: Cell<isize> = const { Cell::new(0) };
    static EVENTS: RefCell<Vec<UiEvent>> = const { RefCell::new(Vec::new()) };
    /// True while this file changes controls itself, so its own edits do not come
    /// back as user input.
    static QUIET: Cell<bool> = const { Cell::new(false) };
}

/// Posted to the dialog after each queued event. Input that arrives as a *sent*
/// message (assistive technology and automation setting a control's text) never
/// makes `GetMessageW` return on its own; this posted message does, so the loop
/// services the queue promptly.
const WM_WAKE_SERVICE: u32 = WM_APP + 10;

fn queue(event: UiEvent) {
    EVENTS.with_borrow_mut(|events| events.push(event));
    if let Some(dialog) = open_dialog() {
        // SAFETY: posting is thread-safe; a full queue only delays servicing to the
        // next message.
        let _ = unsafe { PostMessageW(Some(dialog), WM_WAKE_SERVICE, WPARAM(0), LPARAM(0)) };
    }
}
fn take_events() -> Vec<UiEvent> {
    EVENTS.with_borrow_mut(std::mem::take)
}
fn quietly<R>(f: impl FnOnce() -> R) -> R {
    QUIET.set(true);
    let result = f();
    QUIET.set(false);
    result
}
fn open_dialog() -> Option<HWND> {
    let hwnd = OPEN.get();
    (hwnd != 0).then_some(HWND(hwnd as *mut _))
}

// ---- control helpers (every handle is looked up fresh; none is stored) ----

fn item(parent: HWND, id: i32) -> HWND {
    // SAFETY: plain lookup; a missing control yields a null handle that every
    // helper below tolerates.
    unsafe { GetDlgItem(Some(parent), id) }.unwrap_or_default()
}
fn page(dialog: HWND, page: i32) -> HWND {
    item(dialog, page)
}
fn control(dialog: HWND, page_id: i32, id: i32) -> HWND {
    item(page(dialog, page_id), id)
}
fn send(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> isize {
    // SAFETY: documented control messages with plain-integer or live-pointer
    // arguments; a null or stale handle only fails the call.
    unsafe { SendMessageW(hwnd, message, Some(WPARAM(wparam)), Some(LPARAM(lparam))) }.0
}
fn get_text(hwnd: HWND) -> String {
    // SAFETY: the buffer is sized from the reported length plus the terminator.
    unsafe {
        let length = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buffer = vec![0u16; length + 1];
        let copied = GetWindowTextW(hwnd, &mut buffer).max(0) as usize;
        String::from_utf16_lossy(&buffer[..copied])
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
/// Edit and static controls need CRLF line breaks to show them.
fn control_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}
/// Write only a changed value: rewriting an edit resets its caret and selection.
/// Returns whether the text changed.
fn set_text(hwnd: HWND, text: &str) -> bool {
    let text = control_text(text);
    if get_text(hwnd) == text {
        return false;
    }
    let buffer = wide(&text);
    // SAFETY: NUL-terminated buffer alive for the call.
    let _ = unsafe { SetWindowTextW(hwnd, PCWSTR(buffer.as_ptr())) };
    true
}
fn enable(dialog: HWND, hwnd: HWND, enabled: bool) {
    // SAFETY: simple focus and enable calls on live child windows. A disabled
    // control cannot keep the keyboard, so focus moves on first.
    unsafe {
        if !enabled && GetFocus() == hwnd {
            send(dialog, WM_NEXTDLGCTL, item(dialog, IDOK.0).0 as usize, 1);
        }
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(hwnd, enabled);
    }
}
fn checked(hwnd: HWND, on: bool) {
    send(hwnd, BM_SETCHECK, usize::from(on), 0);
}

fn add_string(combo: HWND, text: &str) {
    let buffer = wide(text);
    send(combo, CB_ADDSTRING, 0, buffer.as_ptr() as isize);
}

/// Tell assistive technology that this status text just changed. Static controls
/// have no live region; an alert event is how a screen reader is told to speak.
fn announce(hwnd: HWND) {
    // SAFETY: notification only; no pointers.
    unsafe { NotifyWinEvent(EVENT_SYSTEM_ALERT, hwnd, OBJID_CLIENT.0, 0) };
}

fn show_selected_page(dialog: HWND) {
    let tab = item(dialog, ID_TAB);
    let selected = send(tab, TCM_GETCURSEL, 0, 0);
    for (index, id) in PAGES.iter().enumerate() {
        // SAFETY: shows or hides our own child dialogs.
        unsafe {
            let _ = ShowWindow(
                page(dialog, *id),
                if index as isize == selected {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
    }
}
fn select_page(dialog: HWND, id: i32) {
    if let Some(index) = PAGES.iter().position(|page| *page == id) {
        send(item(dialog, ID_TAB), TCM_SETCURSEL, index, 0);
        show_selected_page(dialog);
    }
}

/// Place every page over the (hidden) anchor of the template, in client pixels.
fn layout_pages(dialog: HWND) {
    let anchor = item(dialog, ID_ANCHOR);
    let mut rect = RECT::default();
    // SAFETY: reads a live child's rectangle and converts its corner.
    unsafe {
        if GetWindowRect(anchor, &mut rect).is_err() {
            return;
        }
        let mut corner = POINT {
            x: rect.left,
            y: rect.top,
        };
        if !ScreenToClient(dialog, &mut corner).as_bool() {
            return;
        }
        for id in PAGES {
            let _ = SetWindowPos(
                page(dialog, id),
                None,
                corner.x,
                corner.y,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

fn queue_command(wparam: WPARAM) {
    if !QUIET.get() {
        queue(UiEvent::Command {
            id: (wparam.0 & 0xffff) as i32,
            code: ((wparam.0 >> 16) & 0xffff) as u32,
        });
    }
}

// SAFETY contract for both procedures: called synchronously by the dialog manager
// for live windows on the UI thread. They never unwind, allocate fallibly, block
// or keep borrowed state; they only queue values for `SettingsWindow::service`.
unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match message {
        WM_INITDIALOG => {
            OPEN.set(hwnd.0 as isize);
            // SAFETY: shared icons returned by LoadIconW must NOT be destroyed.
            unsafe {
                for size in [ICON_BIG, ICON_SMALL] {
                    SendMessageW(hwnd, WM_SETICON, Some(WPARAM(size as usize)), Some(lparam));
                }
            }
            1 // Dialog manager sets focus to the first tab stop (the page tabs).
        }
        WM_COMMAND => {
            queue_command(wparam);
            1
        }
        WM_NOTIFY => {
            // SAFETY: for WM_NOTIFY lparam points to the sender's NMHDR.
            let header = unsafe { (lparam.0 as *const NMHDR).as_ref() };
            if let Some(header) = header
                && header.idFrom == ID_TAB as usize
                && header.code == TCN_SELCHANGE
            {
                queue(UiEvent::PageChanged);
            }
            0
        }
        // The size changes with the display scale; pages follow their anchor.
        WM_SIZE => {
            queue(UiEvent::Relayout);
            0
        }
        // Close, Escape and the system menu leave the clock running. The window
        // is destroyed by `service`, which also discards the draft.
        WM_CLOSE => {
            queue(UiEvent::Close);
            1
        }
        WM_NCDESTROY => {
            if OPEN.get() == hwnd.0 as isize {
                OPEN.set(0);
            }
            take_events();
            0
        }
        _ => 0, // Default dialog handling includes keyboard and DPI-v2 scaling.
    }
}
unsafe extern "system" fn page_proc(
    _hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    _lparam: LPARAM,
) -> isize {
    match message {
        // Not handled: focus stays where the user left it.
        WM_INITDIALOG => 0,
        WM_COMMAND => {
            queue_command(wparam);
            1
        }
        _ => 0,
    }
}

/// The Settings window and its draft. Modeless: closing it only destroys the
/// window, so the clock keeps running. `Exit` asks the host to quit instead.
pub struct SettingsWindow<S> {
    module: HINSTANCE,
    icon: isize,
    controller: SettingsController<S>,
    /// Result of the last Apply (or a reason it did not happen). Cleared on edit.
    status: String,
    /// Whether this window opened as the first-run welcome.
    welcome: bool,
}

impl<S: PreferencesStore> SettingsWindow<S> {
    pub fn new(session: PreferenceSession<S>) -> Result<Self, PlatformError> {
        // SAFETY: null module name resolves this executable; the initialized
        // stack structure lives for the call; resources are compiled into the app.
        unsafe {
            let module = GetModuleHandleW(None)?;
            let icon = LoadIconW(Some(module.into()), PCWSTR(102_usize as *const u16))?;
            let controls = INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_STANDARD_CLASSES | ICC_TAB_CLASSES | ICC_UPDOWN_CLASS,
            };
            if !InitCommonControlsEx(&controls).as_bool() {
                return Err(windows::core::Error::from_win32().into());
            }
            Ok(Self {
                module: module.into(),
                icon: icon.0 as isize,
                controller: SettingsController::new(session),
                status: String::new(),
                welcome: false,
            })
        }
    }

    pub fn is_open(&self) -> bool {
        OPEN.get() != 0
    }

    /// Open the window, or restore and focus the existing one. Never creates a
    /// second window, and a window that is already open keeps its unsaved edits.
    pub fn show_or_focus<P: Platform>(
        &mut self,
        runtime: &mut Runtime<P>,
    ) -> Result<(), PlatformError> {
        if let Some(existing) = open_dialog() {
            // SAFETY: the stored handle was published by this thread's dialog and
            // is cleared in WM_NCDESTROY.
            unsafe {
                if IsWindow(Some(existing)).as_bool() {
                    if IsIconic(existing).as_bool() {
                        let _ = ShowWindow(existing, SW_RESTORE);
                    }
                    let _ = SetForegroundWindow(existing);
                    return Ok(());
                }
            }
        }
        self.controller.open();
        self.status.clear();
        self.welcome = self.controller.first_run();
        // SAFETY: creation parameters are plain integers; resources are in the exe.
        let dialog = unsafe {
            CreateDialogParamW(
                Some(self.module),
                PCWSTR(DIALOG as *const u16),
                None,
                Some(dialog_proc),
                LPARAM(self.icon),
            )
        }?;
        if let Err(error) = self.build(dialog) {
            // SAFETY: tears down the half-built window we just created.
            unsafe {
                let _ = DestroyWindow(dialog);
            }
            return Err(error);
        }
        self.sync_all(dialog);
        self.refresh(dialog, runtime);
        // SAFETY: shows our own window and brings it forward.
        unsafe {
            let _ = ShowWindow(dialog, SW_SHOW);
            let _ = SetForegroundWindow(dialog);
        }
        Ok(())
    }

    fn build(&self, dialog: HWND) -> Result<(), PlatformError> {
        let tab = item(dialog, ID_TAB);
        let mut after = tab;
        for (index, id) in PAGES.into_iter().enumerate() {
            // SAFETY: child dialogs of our window, created from compiled templates.
            let hwnd = unsafe {
                CreateDialogParamW(
                    Some(self.module),
                    PCWSTR(id as usize as *const u16),
                    Some(dialog),
                    Some(page_proc),
                    LPARAM(0),
                )
            }?;
            // SAFETY: gives the page its id and puts it right after the previous
            // control in z-order, which is also its place in the tab order.
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_ID, id as isize);
                let _ = SetWindowPos(
                    hwnd,
                    Some(after),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            after = hwnd;
            let mut title = wide(PAGE_TITLES[index]);
            let entry = TCITEMW {
                mask: TCIF_TEXT,
                pszText: PWSTR(title.as_mut_ptr()),
                ..Default::default()
            };
            send(
                tab,
                TCM_INSERTITEMW,
                index,
                &entry as *const TCITEMW as isize,
            );
        }
        // The strip's accessible name; the intro text above it is a paragraph.
        let _ = set_text(tab, "Settings sections");
        layout_pages(dialog);
        select_page(dialog, PAGE_DATE);

        let preset = control(dialog, PAGE_DATE, ID_PRESET);
        for p in PRESETS {
            add_string(
                preset,
                &format!("{} ({})", p.name, p.format.replace('\n', " \u{21b5} ")),
            );
        }
        add_string(preset, "Custom format");
        let theme = control(dialog, PAGE_APPEARANCE, ID_THEME);
        for label in ["System (follow Windows)", "Light", "Dark"] {
            add_string(theme, label);
        }
        let font = control(dialog, PAGE_APPEARANCE, ID_FONT);
        send(font, EM_LIMITTEXT, 3, 0);
        let spin = control(dialog, PAGE_APPEARANCE, ID_FONT_SPIN);
        send(spin, UDM_SETBUDDY, font.0 as usize, 0);
        // The arrows are a separate accessible object; give them a name of their own.
        set_text(spin, "Change font size");
        send(
            spin,
            UDM_SETRANGE32,
            usize::from(FONT_MIN),
            isize::from(FONT_MAX),
        );
        set_text(control(dialog, PAGE_DATE, ID_TOKENS), TOKEN_HELP);
        set_text(
            control(dialog, PAGE_STARTUP, ID_STARTUP_INFO),
            "Starting ElevenClock-rs when you sign in to Windows is not available yet. A later \
             update adds an opt-in setting here.\n\nUntil then ElevenClock-rs runs only when you \
             start it, for example from the Start menu, and nothing starts automatically.",
        );
        set_text(
            control(dialog, PAGE_SUPPORT, ID_SUPPORT_INFO),
            &format!(
                "ElevenClock-rs version {}\n\nStatus and a way to copy diagnostics will be added \
                 here in a later update.\n\nChoosing Exit removes the ElevenClock-rs clock and \
                 shows the Windows clock again. The Windows clock itself is never changed.",
                env!("CARGO_PKG_VERSION")
            ),
        );
        let intro = if self.welcome {
            "Welcome to ElevenClock-rs. Choose how the clock looks, then select OK. ElevenClock-rs \
             keeps running when you close this window; use its clock or tray menu to exit."
        } else {
            "ElevenClock-rs keeps running after you close this window. Choose Exit to stop it; \
             the Windows clock is never changed."
        };
        set_text(item(dialog, ID_INTRO), intro);
        Ok(())
    }

    /// Controls from the draft. Used when the code, not the user, changed it.
    fn sync_all(&self, dialog: HWND) {
        let draft = self.controller.draft();
        quietly(|| {
            let format = control(dialog, PAGE_DATE, ID_FORMAT);
            set_text(format, &draft.format);
            set_text(control(dialog, PAGE_APPEARANCE, ID_FONT), &draft.font_size);
            checked(
                control(dialog, PAGE_APPEARANCE, ID_MONITORS_PRIMARY),
                draft.monitors == Monitors::Primary,
            );
            checked(
                control(dialog, PAGE_APPEARANCE, ID_MONITORS_ALL),
                draft.monitors == Monitors::All,
            );
            let theme = match draft.theme {
                Theme::System => 0,
                Theme::Light => 1,
                Theme::Dark => 2,
            };
            send(
                control(dialog, PAGE_APPEARANCE, ID_THEME),
                CB_SETCURSEL,
                theme,
                0,
            );
        });
        // A newer app wrote the saved data: nothing here may be edited or saved.
        let editable = !self.controller.saving_blocked();
        for (page_id, id) in [
            (PAGE_DATE, ID_PRESET),
            (PAGE_DATE, ID_FORMAT),
            (PAGE_APPEARANCE, ID_MONITORS_PRIMARY),
            (PAGE_APPEARANCE, ID_MONITORS_ALL),
            (PAGE_APPEARANCE, ID_FONT),
            (PAGE_APPEARANCE, ID_FONT_SPIN),
            (PAGE_APPEARANCE, ID_THEME),
        ] {
            enable(dialog, control(dialog, page_id, id), editable);
        }
    }

    /// Messages, preview and button states from the draft. Never rewrites an edit
    /// control the user is typing in.
    fn refresh<P: Platform>(&mut self, dialog: HWND, runtime: &mut Runtime<P>) {
        let now = runtime.platform_mut().local_now();
        let evaluation = self.controller.evaluate(runtime, now);
        let problems = &evaluation.problems;
        quietly(|| {
            let preset = evaluation.preset.unwrap_or(PRESETS.len());
            send(
                control(dialog, PAGE_DATE, ID_PRESET),
                CB_SETCURSEL,
                preset,
                0,
            );
            let format_message = problems
                .format
                .as_ref()
                .map(|problem| problem.message.as_str())
                .or(problems.fit.as_deref())
                .unwrap_or("");
            set_text(control(dialog, PAGE_DATE, ID_FORMAT_ERROR), format_message);
            let font_message = problems
                .font
                .as_deref()
                .or(problems.fit.as_deref())
                .unwrap_or("");
            set_text(
                control(dialog, PAGE_APPEARANCE, ID_FONT_ERROR),
                font_message,
            );
            set_text(control(dialog, PAGE_DATE, ID_PREVIEW), &evaluation.preview);
        });
        let blocked = self.controller.saving_blocked();
        enable(dialog, item(dialog, ID_APPLY), evaluation.dirty && !blocked);
        enable(dialog, item(dialog, ID_RESET), !blocked);
        self.show_status(dialog, runtime);
    }

    fn show_status<P: Platform>(&self, dialog: HWND, runtime: &Runtime<P>) {
        let text = if !self.status.is_empty() {
            self.status.clone()
        } else if let Some(notice) = self.controller.notice() {
            notice
        } else if runtime
            .diagnostics()
            .taskbars
            .iter()
            .any(|bar| matches!(bar.reason, Some(Reason::Overflow(_))))
        {
            "The clock is hidden on a taskbar because the current text does not fit. Use a \
             shorter format or a smaller font size."
                .to_owned()
        } else {
            String::new()
        };
        let status = item(dialog, ID_STATUS);
        if set_text(status, &text) && !text.is_empty() {
            announce(status);
        }
    }

    /// Call after every message: handles what the window procedures queued.
    pub fn service<P: Platform>(&mut self, runtime: &mut Runtime<P>) -> Result<(), PlatformError> {
        for event in take_events() {
            let Some(dialog) = open_dialog() else {
                break;
            };
            self.handle(dialog, event, runtime);
        }
        Ok(())
    }

    /// Take the text of both edit controls as the draft. Multi-line edits send no
    /// notification when their text is set from outside (for example by assistive
    /// technology), so the controls, not the notifications, are the source of truth.
    fn pull_inputs(&mut self, dialog: HWND) {
        let format = get_text(control(dialog, PAGE_DATE, ID_FORMAT));
        self.controller.set_format(&format);
        let font = get_text(control(dialog, PAGE_APPEARANCE, ID_FONT));
        self.controller.set_font_size(&font);
    }

    fn handle<P: Platform>(&mut self, dialog: HWND, event: UiEvent, runtime: &mut Runtime<P>) {
        if matches!(event, UiEvent::Command { .. }) {
            self.pull_inputs(dialog);
        }
        let edited = |this: &mut Self, runtime: &mut Runtime<P>| {
            this.status.clear();
            this.refresh(dialog, runtime);
        };
        match event {
            UiEvent::Relayout => layout_pages(dialog),
            UiEvent::PageChanged => show_selected_page(dialog),
            UiEvent::Close => self.finish(dialog),
            UiEvent::Command { id, code } => match (id, code) {
                (id, _) if id == IDCANCEL.0 => self.finish(dialog),
                (id, _) if id == IDOK.0 => self.apply(dialog, runtime, true),
                (ID_APPLY, _) => self.apply(dialog, runtime, false),
                (ID_RESET, _) => {
                    self.controller.reset_to_defaults();
                    self.status.clear();
                    self.sync_all(dialog);
                    self.refresh(dialog, runtime);
                }
                (ID_EXIT, _) => {
                    crate::runtime::native::request_exit();
                    self.finish(dialog);
                }
                (ID_PRESET, code) if code == CBN_SELCHANGE => {
                    let selected = send(control(dialog, PAGE_DATE, ID_PRESET), CB_GETCURSEL, 0, 0);
                    // "Custom format" (the last entry) changes nothing.
                    if usize::try_from(selected).is_ok_and(|i| self.controller.select_preset(i)) {
                        self.sync_all(dialog);
                    }
                    edited(self, runtime);
                }
                (ID_FORMAT, EN_CHANGE) => {
                    let text = get_text(control(dialog, PAGE_DATE, ID_FORMAT));
                    self.controller.set_format(&text);
                    edited(self, runtime);
                }
                (ID_FONT, EN_CHANGE) => {
                    let text = get_text(control(dialog, PAGE_APPEARANCE, ID_FONT));
                    self.controller.set_font_size(&text);
                    edited(self, runtime);
                }
                (ID_MONITORS_PRIMARY, _) => {
                    self.controller.set_monitors(Monitors::Primary);
                    edited(self, runtime);
                }
                (ID_MONITORS_ALL, _) => {
                    self.controller.set_monitors(Monitors::All);
                    edited(self, runtime);
                }
                (ID_THEME, code) if code == CBN_SELCHANGE => {
                    let theme = match send(
                        control(dialog, PAGE_APPEARANCE, ID_THEME),
                        CB_GETCURSEL,
                        0,
                        0,
                    ) {
                        1 => Theme::Light,
                        2 => Theme::Dark,
                        _ => Theme::System,
                    };
                    self.controller.set_theme(theme);
                    edited(self, runtime);
                }
                _ => {}
            },
        }
    }

    /// Apply, and for OK also close. Persists before the clock is told.
    fn apply<P: Platform>(&mut self, dialog: HWND, runtime: &mut Runtime<P>, close: bool) {
        if !self.controller.is_dirty() {
            if close {
                self.finish(dialog);
            }
            return;
        }
        let now = runtime.platform_mut().local_now();
        let mut outcome = self.controller.apply(runtime, now, false);
        if outcome == ApplyOutcome::RecoveryRequired {
            outcome = if confirm_recovery(dialog) {
                self.controller.apply(runtime, now, true)
            } else {
                self.status = "Your damaged saved settings were left as they are.".into();
                self.refresh(dialog, runtime);
                return;
            };
        }
        match outcome {
            ApplyOutcome::Applied(applied) => {
                // Already on disk: only now does the running clock change.
                runtime.handle(Input::Preferences(applied));
                self.controller.finish_first_run();
                if close {
                    self.finish(dialog);
                } else {
                    self.status = "Settings applied.".into();
                    self.sync_all(dialog);
                    self.refresh(dialog, runtime);
                }
            }
            ApplyOutcome::Invalid => {
                self.status = "Settings were not applied. Fix the message shown next to the \
                               highlighted setting."
                    .into();
                self.refresh(dialog, runtime);
                self.focus_problem(dialog, runtime, now);
            }
            ApplyOutcome::NotSaved(message) => {
                self.status = format!("Settings were not saved. {message}");
                self.refresh(dialog, runtime);
            }
            // Handled above; a second confirmation is never needed.
            ApplyOutcome::RecoveryRequired => {}
        }
    }

    /// Send the keyboard to the first setting with a message, selecting the
    /// offending text of a format.
    fn focus_problem<P: Platform>(
        &mut self,
        dialog: HWND,
        runtime: &mut Runtime<P>,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) {
        let problems = self.controller.evaluate(runtime, now).problems;
        let (page_id, id) = match problems.first() {
            Some(Field::FontSize) => (PAGE_APPEARANCE, ID_FONT),
            _ => (PAGE_DATE, ID_FORMAT),
        };
        select_page(dialog, page_id);
        let target = control(dialog, page_id, id);
        // SAFETY: moves focus to a live child control of our own dialog.
        let _ = unsafe { SetFocus(Some(target)) };
        if let Some(selection) = problems
            .format
            .as_ref()
            .and_then(|problem| problem.selection.clone())
            .filter(|_| id == ID_FORMAT)
        {
            send(
                target,
                EM_SETSEL,
                selection.start,
                isize::try_from(selection.end).unwrap_or(isize::MAX),
            );
        } else if id == ID_FONT {
            send(target, EM_SETSEL, 0, -1);
        }
    }

    /// Close without applying: whatever was not applied is discarded.
    fn finish(&mut self, dialog: HWND) {
        self.controller.finish_first_run();
        self.controller.discard();
        self.status.clear();
        // SAFETY: destroys our own dialog on its owning thread.
        unsafe {
            let _ = DestroyWindow(dialog);
        }
    }

    /// Give the dialog manager first refusal on keyboard messages (Tab, arrows,
    /// mnemonics, Enter, Escape). Returns whether the message was consumed.
    pub fn handle_message(&self, message: &mut MSG) -> bool {
        let Some(dialog) = open_dialog() else {
            return false;
        };
        // SAFETY: the dialog handle is live while OPEN is non-null.
        unsafe {
            let ours = message.hwnd == dialog || IsChild(dialog, message.hwnd).as_bool();
            // Ctrl+Tab, Ctrl+Shift+Tab and Ctrl+PageUp/PageDown switch pages, as in
            // property sheets. A multi-line edit would otherwise swallow them.
            if ours && message.message == WM_KEYDOWN && GetKeyState(i32::from(VK_CONTROL.0)) < 0 {
                let key = message.wParam.0;
                let step = if key == usize::from(VK_TAB.0) {
                    Some(if GetKeyState(i32::from(VK_SHIFT.0)) < 0 {
                        PAGES.len() - 1
                    } else {
                        1
                    })
                } else if key == usize::from(VK_NEXT.0) {
                    Some(1)
                } else if key == usize::from(VK_PRIOR.0) {
                    Some(PAGES.len() - 1)
                } else {
                    None
                };
                if let Some(step) = step {
                    let tab = item(dialog, ID_TAB);
                    let current = send(tab, TCM_GETCURSEL, 0, 0).max(0) as usize;
                    send(tab, TCM_SETCURSEL, (current + step) % PAGES.len(), 0);
                    show_selected_page(dialog);
                    let _ = SetFocus(Some(tab));
                    return true;
                }
            }
            IsDialogMessageW(dialog, message).as_bool()
        }
    }

    /// Destroy the window at shutdown. Nothing is saved.
    pub fn close(&mut self) {
        if let Some(dialog) = open_dialog() {
            self.controller.discard();
            // SAFETY: destroys our own dialog on its owning thread.
            let _ = unsafe { DestroyWindow(dialog) };
        }
    }
}

fn confirm_recovery(dialog: HWND) -> bool {
    // SAFETY: modal message box owned by our dialog; static strings.
    unsafe {
        MessageBoxW(
            Some(dialog),
            w!(
                "Your saved ElevenClock-rs settings are damaged. ElevenClock-rs will keep a copy \
                 of the damaged data and save your new settings. Continue?"
            ),
            w!("ElevenClock-rs"),
            MB_YESNO | MB_ICONQUESTION,
        ) == IDYES
    }
}
