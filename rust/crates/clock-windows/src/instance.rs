//! Per-user, per-session single instance and the fixed "show Settings" signal (R08).
//!
//! The legacy app scanned timestamp lockfiles every two seconds. Here a named
//! mutex (`Local\` is session scoped; the user's SID makes it user scoped) decides
//! ownership, and a duplicate launch posts one fixed message to the existing
//! process' hidden host window. A second launch never creates clocks.

/// Window class of the hidden top-level host. It must be top level, not
/// message-only, because `TaskbarCreated`, display, power and setting changes are
/// broadcast only to top-level windows.
pub const HOST_CLASS: &str = "ElevenClock.Runtime.Host";
/// Posted to the host window: open or focus the Settings window.
pub const WM_SHOW_SETTINGS: u32 = 0x8000 + 2;

/// Object and window names derived from the account SID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceNames {
    pub mutex: String,
    pub window_title: String,
}
impl InstanceNames {
    /// SID strings (`S-1-5-21-...`) are valid in kernel object names; anything
    /// else is replaced so a malformed value can never escape the `Local\` scope.
    pub fn for_user(sid: &str) -> Self {
        let clean: String = sid
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
            .collect();
        Self {
            mutex: format!("Local\\ElevenClock.Instance.{clean}"),
            window_title: format!("ElevenClock.Host.{clean}"),
        }
    }
}

/// How this process was started.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchMode {
    /// Start menu, Explorer double-click or any launch without a marker.
    Interactive,
    /// Login startup: run the clock quietly, no Settings window. R12 registers
    /// this exact marker; changing it requires a coordinated edit of both.
    Startup,
}
/// The process role, decided before any single-instance or UI work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Application(LaunchMode),
    /// Private discovery helper (see the helper ADR). Not a user command.
    DiscoveryHelper { generation: u64 },
}
pub const STARTUP_MARKER: &str = "--startup";
pub const HELPER_MARKER: &str = "--elevenclock-discovery-helper";

impl Role {
    /// Unknown arguments are ignored and never printed: the executable has no
    /// public command suite, so Explorer-supplied arguments cannot break launch.
    pub fn from_args<I: IntoIterator<Item = String>>(args: I) -> Self {
        let args: Vec<String> = args.into_iter().collect();
        match args.as_slice() {
            [marker, generation] if marker == HELPER_MARKER => generation
                .parse()
                .map_or(Self::Application(LaunchMode::Interactive), |generation| {
                    Self::DiscoveryHelper { generation }
                }),
            [marker] if marker == STARTUP_MARKER => Self::Application(LaunchMode::Startup),
            _ => Self::Application(LaunchMode::Interactive),
        }
    }
}

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::{Acquired, InstanceGuard, acquire, signal_existing};

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn names_are_user_scoped_and_stay_inside_the_session_namespace() {
        let a = InstanceNames::for_user("S-1-5-21-1-2-3-1001");
        let b = InstanceNames::for_user("S-1-5-21-1-2-3-1002");
        assert_ne!(a, b);
        assert!(a.mutex.starts_with("Local\\"));
        assert!(a.mutex.ends_with("S-1-5-21-1-2-3-1001"));
        let hostile = InstanceNames::for_user("S-1\\..\\Global\\x y");
        assert_eq!(hostile.mutex.matches('\\').count(), 1);
        assert!(!hostile.mutex.contains(' '));
    }

    #[test]
    fn launch_roles_come_from_exact_private_markers_only() {
        let interactive = Role::Application(LaunchMode::Interactive);
        assert_eq!(Role::from_args(args(&[])), interactive);
        assert_eq!(
            Role::from_args(args(&[STARTUP_MARKER])),
            Role::Application(LaunchMode::Startup)
        );
        assert_eq!(
            Role::from_args(args(&[HELPER_MARKER, "7"])),
            Role::DiscoveryHelper { generation: 7 }
        );
        // Malformed, extra and unknown arguments never select a private role.
        for other in [
            &[HELPER_MARKER][..],
            &[HELPER_MARKER, "x"],
            &[HELPER_MARKER, "7", "8"],
            &[STARTUP_MARKER, "extra"],
            &["--help"],
            &["file.txt"],
        ] {
            assert_eq!(Role::from_args(args(other)), interactive, "{other:?}");
        }
    }
}
