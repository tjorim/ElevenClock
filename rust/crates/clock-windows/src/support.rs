//! Support status and the diagnostics report (R11).
//!
//! Everything here is plain values and pure functions so it tests on any host: the
//! plain-language status, the report that Settings previews and copies, redaction
//! of personal data, the bounded wait behind "Refresh status" and the wording of
//! clipboard outcomes. The Windows-only pieces (OS build, the clipboard) are in
//! `native`.
//!
//! There is no diagnostics service and no stable schema: the report is text for a
//! person (or a bug report) to read, and its layout may change between versions.
use crate::runtime::{Diagnostics, Reason, TaskbarDiagnostic};
use chrono::{DateTime, Utc};
use clock_core::{DiscoveryStrategy, HiddenReason, OverlayState};
use std::{sync::OnceLock, time::Duration};

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::{WindowsClipboard, detect_environment};

/// Short commit hash recorded at build time by `build.rs`.
pub const APP_COMMIT: &str = env!("ELEVENCLOCK_RS_COMMIT");

/// Longest "Refresh status" is allowed to look busy. The discovery worker has its
/// own five second deadline; this is the UI-side guard if even that never answers.
pub const REFRESH_LIMIT: Duration = Duration::from_secs(10);
/// Longest piece of a failure reason kept in a report or log line.
const REASON_LIMIT: usize = 200;

/// Facts about the app and the machine, gathered once when Settings opens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Environment {
    pub app_version: String,
    pub commit: String,
    /// For example `10.0.26200.1234 (25H2)`.
    pub windows: String,
    pub os_architecture: String,
    pub app_architecture: String,
}

impl Environment {
    /// Placeholder facts for hosts that are not Windows (tests, type checks).
    pub fn unknown() -> Self {
        Self {
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            commit: APP_COMMIT.to_owned(),
            windows: "unknown".to_owned(),
            os_architecture: "unknown".to_owned(),
            app_architecture: std::env::consts::ARCH.to_owned(),
        }
    }
}

// ---- redaction ---------------------------------------------------------------

/// Removes personal data from free text: the signed-in user's name, their profile
/// folder and any other file path. The report never includes window titles at all,
/// so there is nothing of those to remove.
#[derive(Clone, Debug, Default)]
pub struct Redactor {
    user: Option<String>,
    profile: Option<String>,
}

impl Redactor {
    pub fn new(user: Option<&str>, profile: Option<&str>) -> Self {
        let keep = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        Self {
            user: keep(user),
            profile: keep(profile),
        }
    }

    pub fn from_environment() -> Self {
        Self::new(
            std::env::var("USERNAME").ok().as_deref(),
            std::env::var("USERPROFILE").ok().as_deref(),
        )
    }

    pub fn apply(&self, text: &str) -> String {
        let mut text = text.to_owned();
        if let Some(profile) = &self.profile {
            // The profile folder may contain spaces, so it is matched as a whole
            // and the rest of that path goes with it.
            text = replace_path_prefix(&text, profile);
        }
        text = replace_paths(&text);
        if let Some(user) = &self.user {
            text = replace_ignoring_case(&text, user, "<user>", true);
        }
        text
    }
}

/// The current time in UTC, for the report header.
pub fn now_utc() -> DateTime<Utc> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
        .unwrap_or_default()
}

/// Redact with the current user's name and profile folder.
pub fn redact(text: &str) -> String {
    static REDACTOR: OnceLock<Redactor> = OnceLock::new();
    REDACTOR.get_or_init(Redactor::from_environment).apply(text)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// ASCII case-insensitive replacement (byte offsets stay valid). With `whole_word`
/// the match must not touch a letter or digit, so a user called `al` does not
/// disturb the word `value`.
fn replace_ignoring_case(text: &str, needle: &str, with: &str, whole_word: bool) -> String {
    let haystack = text.to_ascii_lowercase();
    let needle_lower = needle.to_ascii_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut position = 0;
    while let Some(found) = haystack[position..].find(&needle_lower) {
        let start = position + found;
        let end = start + needle_lower.len();
        let before_ok = !whole_word || !text[..start].chars().next_back().is_some_and(is_word);
        let after_ok = !whole_word || !text[end..].chars().next().is_some_and(is_word);
        if before_ok && after_ok {
            result.push_str(&text[position..start]);
            result.push_str(with);
            position = end;
        } else {
            // Not a match: keep one character and look again after it.
            let step = text[start..].chars().next().map_or(1, char::len_utf8);
            result.push_str(&text[position..start + step]);
            position = start + step;
        }
    }
    result.push_str(&text[position..]);
    result
}

/// Characters that end an unquoted path inside a sentence.
fn ends_path(c: char) -> bool {
    c.is_whitespace() || "\"'<>|,;()[]{}".contains(c)
}

/// Replace `prefix` (ASCII case-insensitive) and the rest of that path with `<path>`.
fn replace_path_prefix(text: &str, prefix: &str) -> String {
    let haystack = text.to_ascii_lowercase();
    let needle = prefix.to_ascii_lowercase();
    let mut result = String::with_capacity(text.len());
    let mut position = 0;
    while let Some(found) = haystack[position..].find(&needle) {
        let start = position + found;
        let after = start + needle.len();
        let mut end = text[after..]
            .find(ends_path)
            .map_or(text.len(), |offset| after + offset);
        // Sentence punctuation after a path belongs to the sentence.
        while end > after && text[..end].ends_with(['.', ':', '!', '?']) {
            end -= 1;
        }
        result.push_str(&text[position..start]);
        result.push_str("<path>");
        position = end;
    }
    result.push_str(&text[position..]);
    result
}

/// Replace `C:\...`, `C:/...` and `\\server\share...` with `<path>`.
fn replace_paths(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len());
    let ends = ends_path;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        let after_boundary = index == 0 || !is_word(chars[index - 1]);
        let drive = c.is_ascii_alphabetic()
            && after_boundary
            && chars.get(index + 1) == Some(&':')
            && matches!(chars.get(index + 2), Some('\\' | '/'));
        let unc = c == '\\' && chars.get(index + 1) == Some(&'\\') && after_boundary;
        if drive || unc {
            let mut end = index;
            while end < chars.len() && !ends(chars[end]) {
                end += 1;
            }
            // Sentence punctuation after a path belongs to the sentence.
            while end > index + 1 && matches!(chars[end - 1], '.' | ':' | '!' | '?') {
                end -= 1;
            }
            result.push_str("<path>");
            index = end;
        } else {
            result.push(c);
            index += 1;
        }
    }
    result
}

/// Redact, flatten to one line and cap the length: for reasons from the platform.
fn reason_text(text: &str) -> String {
    let flat: String = redact(text)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.trim();
    match flat.char_indices().nth(REASON_LIMIT) {
        Some((end, _)) => format!("{}...", &flat[..end]),
        None => flat.to_owned(),
    }
}

// ---- status -------------------------------------------------------------------

/// Where replacing the Windows clock stands, for the badge and for tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Replacement {
    /// The ElevenClock-rs clock is showing on every taskbar it manages.
    Active,
    /// Showing on some taskbars; the native clock remains on the others.
    Partial,
    /// Running, but hidden right now by design (full screen, locked, hidden bar).
    Waiting,
    /// The user paused replacement.
    Paused,
    /// The native Windows clock is in use because something went wrong.
    NativeClock,
    /// Still looking for the taskbar.
    Searching,
}
impl Replacement {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Partial => "Active on some taskbars",
            Self::Waiting => "Active, clock hidden for now",
            Self::Paused => "Paused",
            Self::NativeClock => "Windows clock in use",
            Self::Searching => "Looking for the taskbar",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Status {
    pub replacement: Replacement,
    /// One or more sentences that say what is happening and what the user can do.
    pub explanation: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Outcome {
    Replaced,
    /// Hidden on purpose and expected to return by itself.
    Waiting(String),
    /// The native clock is showing because something could not be done.
    Native(String),
    /// A display the user did not choose in Settings: working as intended.
    Unselected,
}

fn outcome(bar: &TaskbarDiagnostic, paused: bool) -> Outcome {
    let native = |text: &str| Outcome::Native(text.to_owned());
    match &bar.reason {
        None if bar.state == OverlayState::Visible => Outcome::Replaced,
        None => Outcome::Waiting("not shown right now".to_owned()),
        Some(Reason::ClockPositionUnverified) => native(
            "ElevenClock-rs could not verify where the Windows clock is on this taskbar, \
             so it leaves the Windows clock alone. This Windows version may not be supported yet",
        ),
        Some(Reason::InvalidGeometry) => native(
            "Windows reported taskbar or display measurements that cannot be used. \
             Changing the display layout or restarting Windows Explorer may fix it",
        ),
        Some(Reason::Overflow(_)) => native(
            "the clock text does not fit this taskbar. Use a shorter format or a smaller \
             font size in Settings",
        ),
        Some(Reason::RenderFailed(why)) => Outcome::Native(format!(
            "the clock could not be drawn ({})",
            reason_text(why)
        )),
        Some(Reason::PlacementFailed(why)) => Outcome::Native(format!(
            "the clock window could not be placed on this taskbar ({})",
            reason_text(why)
        )),
        Some(Reason::ActionFailed(why)) => Outcome::Native(format!(
            "opening the calendar failed ({}), so the Windows clock was restored. Choose \
             Pause and then Resume from the clock menu to try again",
            reason_text(why)
        )),
        Some(Reason::NotShown) => match bar.state {
            OverlayState::Hidden(HiddenReason::TaskbarHidden) => {
                Outcome::Waiting("the taskbar is hidden".to_owned())
            }
            OverlayState::Hidden(HiddenReason::Fullscreen) => {
                Outcome::Waiting("a full-screen app is in front".to_owned())
            }
            OverlayState::Hidden(HiddenReason::SessionLocked) => {
                Outcome::Waiting("the PC is locked".to_owned())
            }
            OverlayState::Hidden(HiddenReason::Paused) if paused => {
                Outcome::Waiting("replacement is paused".to_owned())
            }
            OverlayState::Hidden(HiddenReason::Paused) => Outcome::Unselected,
            OverlayState::Hidden(HiddenReason::InvalidLayout) | OverlayState::Unavailable => {
                native("the taskbar layout could not be used")
            }
            OverlayState::Visible => Outcome::Waiting("not shown right now".to_owned()),
        },
    }
}

/// One short phrase for what a taskbar is doing, for the report and the log.
pub fn taskbar_summary(bar: &TaskbarDiagnostic, paused: bool) -> String {
    match outcome(bar, paused) {
        Outcome::Replaced => "clock replaced".to_owned(),
        Outcome::Waiting(why) => format!("clock hidden for now: {why}"),
        Outcome::Native(why) => format!("Windows clock in use: {why}"),
        Outcome::Unselected => {
            "not used: this display is not selected in Settings (Appearance)".to_owned()
        }
    }
}

fn discovery_failure_text(diagnostics: &Diagnostics) -> Option<String> {
    diagnostics
        .last_discovery_failure
        .as_ref()
        .map(|failure| reason_text(&failure.to_string()))
}

/// The plain-language status Settings shows, with an actionable explanation
/// whenever the native clock is in use.
pub fn status(diagnostics: &Diagnostics) -> Status {
    let make = |replacement, explanation: String| Status {
        replacement,
        explanation,
    };
    if diagnostics.discovery_disabled {
        return make(
            Replacement::NativeClock,
            "ElevenClock-rs stopped looking for the taskbar after a helper program could \
             not be stopped safely. The Windows clock is showing and was not changed. \
             Exit and start ElevenClock-rs again to retry."
                .to_owned(),
        );
    }
    if diagnostics.paused {
        return make(
            Replacement::Paused,
            "Clock replacement is paused, so the Windows clock is showing. Choose Resume \
             from the clock or tray menu to bring the ElevenClock-rs clock back."
                .to_owned(),
        );
    }
    // Displays the user left out in Settings are a choice, not a problem: they are
    // mentioned, but never count as "some taskbars" failing.
    let all: Vec<Outcome> = diagnostics
        .taskbars
        .iter()
        .map(|bar| outcome(bar, diagnostics.paused))
        .collect();
    let unselected = all
        .iter()
        .filter(|outcome| matches!(outcome, Outcome::Unselected))
        .count();
    let outcomes: Vec<(usize, &Outcome)> = all
        .iter()
        .enumerate()
        .filter(|(_, outcome)| !matches!(outcome, Outcome::Unselected))
        .collect();
    let total = outcomes.len();
    let replaced = outcomes
        .iter()
        .filter(|(_, outcome)| **outcome == Outcome::Replaced)
        .count();
    let mut notes: Vec<String> = Vec::new();
    for (index, outcome) in &outcomes {
        let note = match outcome {
            Outcome::Replaced | Outcome::Unselected => continue,
            Outcome::Waiting(text) | Outcome::Native(text) => text,
        };
        let line = if total > 1 {
            let bar = &diagnostics.taskbars[*index];
            format!(
                "{}: {note}.",
                if bar.primary {
                    "Primary taskbar".to_owned()
                } else {
                    format!("Taskbar {}", index + 1)
                }
            )
        } else {
            capitalised(&format!("{note}."))
        };
        if !notes.contains(&line) {
            notes.push(line);
        }
    }
    if unselected > 0 {
        notes.push(
            "Other displays keep the Windows clock because Settings shows the clock on the \
             primary display only."
                .to_owned(),
        );
    }
    let any_native = outcomes
        .iter()
        .any(|(_, outcome)| matches!(outcome, Outcome::Native(_)));
    let tail = notes.join(" ");
    if total > 0 && replaced == total {
        return make(
            Replacement::Active,
            format!(
                "The ElevenClock-rs clock is showing on {}. {tail}",
                if total == 1 {
                    "your taskbar".to_owned()
                } else {
                    format!("all {total} taskbars")
                }
            )
            .trim_end()
            .to_owned(),
        );
    }
    if replaced > 0 {
        return make(
            Replacement::Partial,
            format!(
                "The ElevenClock-rs clock is showing on {replaced} of {total} taskbars. \
                 The Windows clock remains on the others. {tail}"
            ),
        );
    }
    if total > 0 {
        return if any_native {
            make(
                Replacement::NativeClock,
                format!("The Windows clock is showing and was not changed. {tail}"),
            )
        } else {
            make(
                Replacement::Waiting,
                format!(
                    "ElevenClock-rs is running. Its clock is hidden for now and returns by \
                     itself. {tail}"
                ),
            )
        };
    }
    // No taskbar at all.
    if let Some(failure) = discovery_failure_text(diagnostics) {
        return make(
            Replacement::NativeClock,
            format!(
                "ElevenClock-rs could not find the taskbar, so the Windows clock is showing \
                 and was not changed. {failure} It tries again automatically; Refresh status \
                 tries right now."
            ),
        );
    }
    if !diagnostics.shell_running {
        return make(
            Replacement::NativeClock,
            "Windows Explorer is not running, so there is no taskbar to put the clock on. \
             ElevenClock-rs resumes by itself when Explorer starts."
                .to_owned(),
        );
    }
    make(
        Replacement::Searching,
        "ElevenClock-rs is looking for the taskbar. The Windows clock is showing until it \
         is found."
            .to_owned(),
    )
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// The status block shown at the top of the Support page.
pub fn status_text(environment: &Environment, diagnostics: &Diagnostics) -> String {
    let status = status(diagnostics);
    format!(
        "ElevenClock-rs version {}\nWindows {}\nStatus: {}\n{}",
        environment.app_version,
        environment.windows,
        status.replacement.label(),
        status.explanation,
    )
}

// ---- report ------------------------------------------------------------------

fn strategy_name(strategy: DiscoveryStrategy) -> &'static str {
    match strategy {
        DiscoveryStrategy::NativeChildren => "native child windows",
        DiscoveryStrategy::UiAutomation => "UI Automation",
    }
}

/// DPI as the familiar scale, for example `144 (150%)`.
fn dpi_text(dpi: u32) -> String {
    if dpi == 0 {
        "unknown".to_owned()
    } else {
        format!("{dpi} ({}%)", (u64::from(dpi) * 100 + 48) / 96)
    }
}

/// The human-readable report. Contains no user name, file path or window title;
/// free-text reasons are redacted again as they are written.
pub fn report(
    environment: &Environment,
    diagnostics: &Diagnostics,
    generated: DateTime<Utc>,
) -> String {
    let status = status(diagnostics);
    let mut out = String::new();
    let mut line = |text: String| {
        out.push_str(&text);
        out.push('\n');
    };
    line("ElevenClock-rs diagnostics".to_owned());
    line(format!(
        "Generated (UTC): {}",
        generated.format("%Y-%m-%d %H:%M:%S")
    ));
    line(
        "Contains no user name, file paths or window titles. Nothing is sent anywhere; \
         this text is only copied if you choose Copy diagnostics."
            .to_owned(),
    );
    line(String::new());
    line(format!("App version: {}", environment.app_version));
    line(format!("App commit: {}", environment.commit));
    line(format!("Windows: {}", environment.windows));
    line(format!(
        "Architecture: Windows {}, app {}",
        environment.os_architecture, environment.app_architecture
    ));
    line(String::new());
    line(format!("Clock replacement: {}", status.replacement.label()));
    line(format!("Summary: {}", status.explanation));
    line(format!(
        "Paused by user: {}",
        if diagnostics.paused { "yes" } else { "no" }
    ));
    line(format!(
        "Windows Explorer running: {}",
        if diagnostics.shell_running {
            "yes"
        } else {
            "no"
        }
    ));
    let discovery = if diagnostics.discovery_disabled {
        "turned off for this session".to_owned()
    } else if let Some(failure) = discovery_failure_text(diagnostics) {
        format!(
            "last attempt failed: {failure} ({} in a row)",
            diagnostics.consecutive_failures
        )
    } else {
        "last attempt succeeded".to_owned()
    };
    line(format!("Taskbar discovery: {discovery}"));
    line(String::new());
    line(format!("Taskbars: {}", diagnostics.taskbars.len()));
    for (index, bar) in diagnostics.taskbars.iter().enumerate() {
        let result = taskbar_summary(bar, diagnostics.paused);
        line(format!(
            "{}. {} taskbar: {result}",
            index + 1,
            if bar.primary { "Primary" } else { "Secondary" }
        ));
        let monitor = bar.monitor_rect;
        line(format!(
            "   Monitor: {}x{} px at ({}, {}), DPI {}",
            monitor.width(),
            monitor.height(),
            monitor.left(),
            monitor.top(),
            dpi_text(bar.dpi),
        ));
        line(format!(
            "   Discovery strategy: {}; clock located: {}",
            strategy_name(bar.strategy),
            if bar.clock_located { "yes" } else { "no" }
        ));
    }
    out
}

// ---- refresh --------------------------------------------------------------------

/// What "Refresh status" is waiting for. Created when the user asks; polled by
/// Settings with the time waited so far.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefreshWatch {
    completed_before: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Progress {
    Waiting,
    /// A new discovery result is in (or discovery is off, so none will come).
    Done,
    /// Nothing arrived in [`REFRESH_LIMIT`]: show what is known and stop waiting.
    TimedOut,
}
impl RefreshWatch {
    pub fn start(diagnostics: &Diagnostics) -> Self {
        Self {
            completed_before: diagnostics.discovery_completed,
        }
    }

    pub fn progress(&self, diagnostics: &Diagnostics, waited: Duration) -> Progress {
        let answered = diagnostics.discovery_completed > self.completed_before
            && !diagnostics.discovery_pending;
        if answered || diagnostics.discovery_disabled {
            Progress::Done
        } else if waited >= REFRESH_LIMIT {
            Progress::TimedOut
        } else {
            Progress::Waiting
        }
    }
}

// ---- clipboard outcomes -------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardFault {
    /// Another program kept the clipboard open for the whole bounded wait.
    Busy,
    /// The text could not be prepared.
    OutOfMemory,
    /// Windows refused the text.
    Rejected,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipboardError {
    pub fault: ClipboardFault,
    /// True only if the old contents were already cleared when this happened.
    pub replaced: bool,
}

/// Where the report goes. Settings uses the real clipboard; tests use a fake.
pub trait Clipboard {
    fn set_text(&mut self, text: &str) -> Result<(), ClipboardError>;
}

/// Accessible wording for the result of Copy diagnostics.
pub fn copy_message(result: Result<(), ClipboardError>) -> String {
    match result {
        Ok(()) => "Diagnostics copied to the clipboard.".to_owned(),
        Err(error) => {
            let why = match error.fault {
                ClipboardFault::Busy => {
                    "Another program is using the clipboard. Wait a moment and choose Copy \
                     diagnostics again."
                }
                ClipboardFault::OutOfMemory => "Windows is low on memory.",
                ClipboardFault::Rejected => "Windows did not accept the text.",
            };
            let effect = if error.replaced {
                "The clipboard was cleared."
            } else {
                "Your clipboard was not changed."
            };
            format!("Diagnostics were not copied. {why} {effect}")
        }
    }
}

#[cfg(test)]
mod tests;
