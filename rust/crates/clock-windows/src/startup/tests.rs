use super::*;
use std::{cell::RefCell, rc::Rc};

fn exe() -> PathBuf {
    PathBuf::from(r"C:\Tools\ElevenClock-rs\elevenclock-rs.exe")
}
fn entry(command: Option<&str>, approved: bool) -> RunEntry {
    RunEntry {
        command: command.map(str::to_owned),
        approved,
    }
}
/// `Path::is_absolute` is a Windows notion; these tests feed Windows paths, so on
/// other hosts they only exercise the portable string rules.
#[cfg(windows)]
fn absolute(path: &str) -> &Path {
    Path::new(path)
}

#[test]
fn value_name_never_collides_with_the_legacy_app() {
    assert!(!VALUE_NAME.eq_ignore_ascii_case("elevenClock"));
}

#[cfg(windows)]
#[test]
fn command_quotes_the_absolute_path_and_adds_the_private_marker() {
    assert_eq!(
        command_line(&exe()).unwrap(),
        r#""C:\Tools\ElevenClock-rs\elevenclock-rs.exe" --startup"#
    );
}

#[cfg(windows)]
#[test]
fn spaces_and_unicode_round_trip_through_the_command() {
    for path in [
        r"C:\Program Files\Eleven Clock\elevenclock-rs.exe",
        r"C:\Users\Zoë Müller\Apps 日本語\elevenclock-rs.exe",
        r"\\server\share\my apps\elevenclock-rs.exe",
        r"D:\a b  c\elevenclock-rs.exe",
    ] {
        let command = command_line(absolute(path)).unwrap();
        assert_eq!(split_command(&command), Some((path, ARGUMENT)), "{path}");
        assert_eq!(
            reconcile(&entry(Some(&command), true), absolute(path), |_| true),
            StartupState::On,
            "{path}"
        );
    }
}

#[cfg(windows)]
#[test]
fn paths_that_cannot_be_quoted_are_refused() {
    assert_eq!(
        command_line(Path::new("elevenclock-rs.exe")),
        Err(CommandError::NotAbsolute)
    );
    assert_eq!(
        command_line(Path::new(r"..\x\elevenclock-rs.exe")),
        Err(CommandError::NotAbsolute)
    );
    assert_eq!(
        command_line(Path::new("C:\\bad\"name\\a.exe")),
        Err(CommandError::Unquotable)
    );
    assert_eq!(
        command_line(Path::new("C:\\bad\nname\\a.exe")),
        Err(CommandError::Unquotable)
    );
    let long = format!("C:\\{}\\a.exe", "x".repeat(COMMAND_LIMIT));
    assert_eq!(command_line(Path::new(&long)), Err(CommandError::TooLong));
}

#[test]
fn split_follows_the_shell_rules() {
    assert_eq!(
        split_command(r#""C:\a b\c.exe" --startup"#),
        Some((r"C:\a b\c.exe", "--startup"))
    );
    assert_eq!(
        split_command(r#""C:\a b\c.exe""#),
        Some((r"C:\a b\c.exe", ""))
    );
    assert_eq!(
        split_command(r"C:\ab\c.exe --startup"),
        Some((r"C:\ab\c.exe", "--startup"))
    );
    assert_eq!(split_command(r#""C:\unterminated --startup"#), None);
    assert_eq!(split_command(r#""" x"#), None);
    assert_eq!(split_command("   "), None);
}

#[test]
fn no_value_means_off() {
    assert_eq!(
        reconcile(&entry(None, true), &exe(), |_| true),
        StartupState::Off
    );
}

#[test]
fn matching_value_is_on_and_path_case_does_not_matter() {
    let command = format!("\"{}\" {ARGUMENT}", exe().display());
    assert_eq!(
        reconcile(&entry(Some(&command), true), &exe(), |_| true),
        StartupState::On
    );
    let shouty = command.to_uppercase().replace("--STARTUP", ARGUMENT);
    assert_eq!(
        reconcile(&entry(Some(&shouty), true), &exe(), |_| true),
        StartupState::On
    );
}

#[test]
fn windows_switching_the_entry_off_is_reported_not_hidden() {
    let command = format!("\"{}\" {ARGUMENT}", exe().display());
    assert_eq!(
        reconcile(&entry(Some(&command), false), &exe(), |_| true),
        StartupState::DisabledInWindows
    );
}

#[test]
fn a_moved_portable_copy_is_stale_and_missing() {
    let old = r#""D:\Portable\elevenclock-rs.exe" --startup"#;
    assert_eq!(
        reconcile(&entry(Some(old), true), &exe(), |_| false),
        StartupState::Stale(Stale::Missing {
            registered: r"D:\Portable\elevenclock-rs.exe".into()
        })
    );
}

#[test]
fn another_existing_copy_is_reported_as_such() {
    let other = r#""D:\Other\elevenclock-rs.exe" --startup"#;
    assert_eq!(
        reconcile(&entry(Some(other), true), &exe(), |_| true),
        StartupState::Stale(Stale::OtherCopy {
            registered: r"D:\Other\elevenclock-rs.exe".into()
        })
    );
}

#[test]
fn foreign_shapes_are_unrecognised_even_for_the_same_program() {
    let unquoted = format!("{} {ARGUMENT}", exe().display());
    for command in [
        "",
        "   ",
        r#""C:\Tools\ElevenClock-rs\elevenclock-rs.exe""#,
        r#""C:\Tools\ElevenClock-rs\elevenclock-rs.exe" --other"#,
        r#""C:\Tools\ElevenClock-rs\elevenclock-rs.exe" --startup extra"#,
        r#""unterminated --startup"#,
        unquoted.as_str(),
    ] {
        assert_eq!(
            reconcile(&entry(Some(command), true), &exe(), |_| true),
            StartupState::Stale(Stale::Unrecognised),
            "{command:?}"
        );
    }
}

/// An in-memory Run key that can be told to fail.
#[derive(Clone, Default)]
struct Fake(Rc<RefCell<FakeState>>);
#[derive(Default)]
struct FakeState {
    command: Option<String>,
    fail_write: bool,
    fail_remove: bool,
    fail_read: bool,
    writes: usize,
}
impl RunKey for Fake {
    fn read(&mut self) -> Result<RunEntry, StartupError> {
        let state = self.0.borrow();
        if state.fail_read {
            return Err(StartupError::Registry {
                action: "read",
                detail: "denied".into(),
            });
        }
        Ok(entry(state.command.as_deref(), true))
    }
    fn write(&mut self, command: &str) -> Result<(), StartupError> {
        let mut state = self.0.borrow_mut();
        if state.fail_write {
            return Err(StartupError::Registry {
                action: "turn on startup",
                detail: "denied".into(),
            });
        }
        state.command = Some(command.to_owned());
        state.writes += 1;
        Ok(())
    }
    fn remove(&mut self) -> Result<(), StartupError> {
        let mut state = self.0.borrow_mut();
        if state.fail_remove {
            return Err(StartupError::Registry {
                action: "turn off startup",
                detail: "denied".into(),
            });
        }
        state.command = None;
        Ok(())
    }
}

#[cfg(windows)]
#[test]
fn enable_then_disable_round_trips_and_state_is_read_back() {
    let fake = Fake::default();
    let mut controller = StartupController::new(fake.clone(), exe());
    assert_eq!(controller.state().unwrap(), StartupState::Off);
    controller.enable().unwrap();
    assert_eq!(
        fake.0.borrow().command.as_deref(),
        Some(command_line(&exe()).unwrap().as_str())
    );
    assert_eq!(controller.state().unwrap(), StartupState::On);
    controller.disable().unwrap();
    assert_eq!(controller.state().unwrap(), StartupState::Off);
    controller.disable().unwrap();
}

#[cfg(windows)]
#[test]
fn a_failed_write_leaves_the_registry_and_the_reported_state_unchanged() {
    let fake = Fake::default();
    fake.0.borrow_mut().fail_write = true;
    let mut controller = StartupController::new(fake.clone(), exe());
    let error = controller.enable().unwrap_err();
    assert!(
        error.message().contains("did not let ElevenClock-rs"),
        "{}",
        error.message()
    );
    assert_eq!(controller.state().unwrap(), StartupState::Off);
    assert_eq!(fake.0.borrow().writes, 0);
}

#[cfg(windows)]
#[test]
fn a_failed_removal_keeps_reporting_the_registration_that_still_exists() {
    let fake = Fake::default();
    let mut controller = StartupController::new(fake.clone(), exe());
    controller.enable().unwrap();
    fake.0.borrow_mut().fail_remove = true;
    assert!(controller.disable().is_err());
    assert_eq!(controller.state().unwrap(), StartupState::On);
}

#[test]
fn an_unreadable_key_is_an_error_never_off() {
    let fake = Fake::default();
    fake.0.borrow_mut().fail_read = true;
    let mut controller = StartupController::new(fake, exe());
    assert!(controller.state().is_err());
}

#[cfg(windows)]
#[test]
fn repair_replaces_a_stale_entry_only_when_asked() {
    let fake = Fake::default();
    fake.0.borrow_mut().command = Some(r#""D:\Gone\elevenclock-rs.exe" --startup"#.into());
    let mut controller = StartupController::new(fake.clone(), exe());
    // Reading never rewrites.
    assert!(matches!(
        controller.state().unwrap(),
        StartupState::Stale(Stale::Missing { .. })
    ));
    assert_eq!(fake.0.borrow().writes, 0);
    controller.repair().unwrap();
    assert_eq!(controller.state().unwrap(), StartupState::On);
}

#[test]
fn an_unusable_executable_path_is_refused_before_touching_the_registry() {
    let fake = Fake::default();
    let mut controller = StartupController::new(fake.clone(), PathBuf::from("relative.exe"));
    assert!(matches!(
        controller.enable(),
        Err(StartupError::Command(CommandError::NotAbsolute))
    ));
    assert_eq!(fake.0.borrow().writes, 0);
}

#[test]
fn every_state_has_words_and_stale_ones_name_the_repair() {
    for state in [
        StartupState::Off,
        StartupState::On,
        StartupState::DisabledInWindows,
        StartupState::Stale(Stale::Missing {
            registered: "X".into(),
        }),
        StartupState::Stale(Stale::OtherCopy {
            registered: "Y".into(),
        }),
        StartupState::Stale(Stale::Unrecognised),
    ] {
        let text = describe(&state);
        assert!(!text.is_empty());
        assert_eq!(
            matches!(state, StartupState::Stale(_)),
            text.contains("Repair startup entry"),
            "{text}"
        );
    }
}
