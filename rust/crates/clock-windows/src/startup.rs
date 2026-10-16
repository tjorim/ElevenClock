//! Opt-in login startup (R12).
//!
//! The registration is a single per-user value under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, owned by this app and named
//! [`VALUE_NAME`] (the legacy ElevenClock uses `elevenClock`; registry value names are
//! case-insensitive, so ours must differ by more than case). Nothing here needs
//! administrator rights. The only things ever written are that one value and the
//! same-named `StartupApproved` flag Windows keeps for it.
//!
//! The registry is the only source of truth: no preference stores whether startup is
//! on, so the checkbox can never drift from what Windows will really do. Everything
//! that is a rule is plain values and pure functions, tested on any host:
//!
//! * [`command_line`] builds the quoted absolute command and refuses paths that
//!   cannot be quoted safely;
//! * [`reconcile`] turns the registered text and Windows' own enabled flag into a
//!   [`StartupState`] for *this* executable;
//! * [`StartupController`] enables, disables and repairs through a [`RunKey`]; the
//!   caller reads the state back afterwards, so a failed write leaves the UI showing
//!   what the OS actually holds.
//!
//! Only an explicit user action changes the registry. A stale or moved registration
//! is reported and offered for repair; it is never rewritten on launch.
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::{RegistryRunKey, current_executable};

/// The app's exact `Run` value name. Distinct from the legacy `elevenClock`.
pub const VALUE_NAME: &str = "ElevenClockRust";
pub const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Windows records that the user switched an entry off (Task Manager, Settings >
/// Apps > Startup) in a same-named binary value under this key.
pub const APPROVED_SUBKEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
/// The single argument the registered command passes (see `instance::Role`).
pub const ARGUMENT: &str = crate::instance::STARTUP_MARKER;

/// Longest command Windows accepts in a Run value, in UTF-16 units.
const COMMAND_LIMIT: usize = 32_767;

/// Why a registration cannot be written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandError {
    /// A Run command needs a full path: a relative one depends on Windows' logon directory.
    NotAbsolute,
    /// A double quote cannot appear in a Windows path and would break the quoting;
    /// a control character cannot be stored in the value.
    Unquotable,
    TooLong,
}

/// The exact command Windows should run at logon: the quoted absolute executable
/// path and the private startup marker.
pub fn command_line(executable: &Path) -> Result<String, CommandError> {
    let text = executable.to_str().ok_or(CommandError::Unquotable)?;
    if !executable.is_absolute() {
        return Err(CommandError::NotAbsolute);
    }
    if text.chars().any(|c| c == '"' || c.is_control()) {
        return Err(CommandError::Unquotable);
    }
    let command = format!("\"{text}\" {ARGUMENT}");
    if command.encode_utf16().count() >= COMMAND_LIMIT {
        return Err(CommandError::TooLong);
    }
    Ok(command)
}

/// Splits a registered command into its program and the remaining arguments, the way
/// the shell reads a Run value: a leading quoted path, or text up to the first space.
/// `None` for empty text or an unterminated quote.
pub fn split_command(command: &str) -> Option<(&str, &str)> {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix('"') {
        let end = rest.find('"')?;
        let program = &rest[..end];
        (!program.is_empty()).then_some((program, rest[end + 1..].trim()))
    } else {
        let (program, arguments) = command
            .split_once(char::is_whitespace)
            .unwrap_or((command, ""));
        (!program.is_empty()).then_some((program, arguments.trim()))
    }
}

#[cfg(windows)]
fn wide_nul(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Windows compares paths without regard to case.
fn same_path(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// What the registry holds for this app's value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunEntry {
    /// The stored text, or `None` when the value is absent.
    pub command: Option<String>,
    /// `false` when the user switched the entry off in Windows.
    pub approved: bool,
}

/// How the registration relates to this executable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartupState {
    /// No registration: nothing starts at logon.
    Off,
    /// Registered for exactly this executable, and Windows will run it.
    On,
    /// Registered for this executable, but turned off in Windows' own startup list.
    DisabledInWindows,
    /// Registered, but not for this executable. Needs an explicit repair.
    Stale(Stale),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Stale {
    /// The registered program no longer exists (a portable copy was moved or deleted).
    Missing { registered: String },
    /// The registered program exists but is another copy of the app.
    OtherCopy { registered: String },
    /// The value is not a command this app writes.
    Unrecognised,
}

/// Decide the state of `entry` for the running executable. `exists` is injected so
/// the rules are testable without a file system.
pub fn reconcile(
    entry: &RunEntry,
    executable: &Path,
    exists: impl Fn(&str) -> bool,
) -> StartupState {
    let Some(command) = entry.command.as_deref() else {
        return StartupState::Off;
    };
    // Only the exact quoted form this app writes counts as recognised: an unquoted
    // path with spaces would be split by Windows at the first space.
    let recognised = command.trim_start().starts_with('"');
    let Some((program, arguments)) = split_command(command).filter(|_| recognised) else {
        return StartupState::Stale(Stale::Unrecognised);
    };
    if arguments != ARGUMENT {
        return StartupState::Stale(Stale::Unrecognised);
    }
    if same_path(program, &executable.to_string_lossy()) {
        return if entry.approved {
            StartupState::On
        } else {
            StartupState::DisabledInWindows
        };
    }
    let registered = program.to_owned();
    if exists(program) {
        StartupState::Stale(Stale::OtherCopy { registered })
    } else {
        StartupState::Stale(Stale::Missing { registered })
    }
}

/// A failure the user can be told about, in words. `detail` is the OS reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartupError {
    Command(CommandError),
    /// The registry could not be read or written.
    Registry {
        action: &'static str,
        detail: String,
    },
}
impl StartupError {
    pub fn message(&self) -> String {
        match self {
            Self::Command(CommandError::NotAbsolute | CommandError::Unquotable) => {
                "ElevenClock-rs cannot start at sign-in from this location: its path cannot \
                 be used in a Windows startup entry."
                    .to_owned()
            }
            Self::Command(CommandError::TooLong) => {
                "ElevenClock-rs cannot start at sign-in from this location: its path is too \
                 long for a Windows startup entry."
                    .to_owned()
            }
            Self::Registry { action, detail } => {
                format!("Windows did not let ElevenClock-rs {action} ({detail}).")
            }
        }
    }
}

/// The per-user Run registration, as a trait so tests supply a fake and the native
/// side can target a temporary key.
pub trait RunKey {
    fn read(&mut self) -> Result<RunEntry, StartupError>;
    /// Store the command as this app's Run value and clear a "turned off" flag
    /// Windows holds for that same value.
    fn write(&mut self, command: &str) -> Result<(), StartupError>;
    /// Remove this app's value and its Windows flag. Absent is success.
    fn remove(&mut self) -> Result<(), StartupError>;
}

/// What the Startup page reads and does, built over a [`RunKey`].
pub struct StartupController<K> {
    key: K,
    executable: PathBuf,
}
impl<K: RunKey> StartupController<K> {
    pub fn new(key: K, executable: PathBuf) -> Self {
        Self { key, executable }
    }

    /// The state in the registry right now.
    pub fn state(&mut self) -> Result<StartupState, StartupError> {
        let entry = self.key.read()?;
        Ok(reconcile(&entry, &self.executable, |program| {
            Path::new(program).is_file()
        }))
    }

    /// Register this executable (also turns back on an entry Windows had switched off).
    pub fn enable(&mut self) -> Result<(), StartupError> {
        let command = command_line(&self.executable).map_err(StartupError::Command)?;
        self.key.write(&command)
    }
    pub fn disable(&mut self) -> Result<(), StartupError> {
        self.key.remove()
    }
    /// Re-point a stale entry at this executable. The same write as `enable`, kept
    /// as its own call so the window can only repair on the user's explicit request.
    pub fn repair(&mut self) -> Result<(), StartupError> {
        self.enable()
    }
}

/// The page's words for a state; one place so the tests and the window agree.
pub fn describe(state: &StartupState) -> String {
    match state {
        StartupState::Off => "ElevenClock-rs does not start when you sign in.".to_owned(),
        StartupState::On => "ElevenClock-rs starts when you sign in to Windows.".to_owned(),
        StartupState::DisabledInWindows => "ElevenClock-rs is registered to start when you \
            sign in, but Windows has it turned off. Select the checkbox to turn it back on, \
            or use Windows Settings, Apps, Startup."
            .to_owned(),
        StartupState::Stale(Stale::Missing { registered }) => format!(
            "The startup entry points to a program that no longer exists:\n{registered}\n\
             Nothing starts at sign-in. Choose Repair startup entry to use this copy of \
             ElevenClock-rs, or clear the checkbox to remove the entry."
        ),
        StartupState::Stale(Stale::OtherCopy { registered }) => format!(
            "The startup entry starts a different copy of ElevenClock-rs:\n{registered}\n\
             Choose Repair startup entry to use this copy instead, or clear the checkbox to \
             remove the entry."
        ),
        StartupState::Stale(Stale::Unrecognised) => "The startup entry is not one \
            ElevenClock-rs recognises. Choose Repair startup entry to replace it, or clear \
            the checkbox to remove it."
            .to_owned(),
    }
}

#[cfg(test)]
mod tests;
