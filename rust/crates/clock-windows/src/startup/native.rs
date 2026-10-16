//! The real per-user Run key. Only this app's exact value is ever read for writing,
//! written or deleted; HKLM, services and scheduled tasks are never involved.
use super::{APPROVED_SUBKEY, RUN_SUBKEY, RunEntry, RunKey, StartupError, VALUE_NAME, wide_nul};
use std::path::PathBuf;
use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PATH_NOT_FOUND, WIN32_ERROR},
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_BINARY, REG_EXPAND_SZ,
            REG_OPEN_CREATE_OPTIONS, REG_SZ, REG_VALUE_TYPE, RRF_NOEXPAND, RRF_RT_ANY, RegCloseKey,
            RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
        },
    },
    core::{HRESULT, PCWSTR},
};

/// The running executable as an absolute path, without a `\\?\` prefix that a Run
/// command could not use.
pub fn current_executable() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?;
    let text = path.to_str()?;
    Some(match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    })
}

fn describe(code: WIN32_ERROR) -> String {
    let error = windows::core::Error::from_hresult(HRESULT::from_win32(code.0));
    let message = error.message();
    let message = message.trim();
    if message.is_empty() {
        format!("error {}", code.0)
    } else {
        format!("{message} - error {}", code.0)
    }
}
fn failure(action: &'static str, code: WIN32_ERROR) -> StartupError {
    StartupError::Registry {
        action,
        detail: describe(code),
    }
}
fn absent(code: WIN32_ERROR) -> bool {
    code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND
}

/// Registry text is little-endian UTF-16, possibly with a terminator.
fn text_from_bytes(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// A key handle that is always closed.
struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: closes the handle this value opened; it is not used afterwards.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// Reads a value's type and raw bytes. `None` when the key or value is absent.
fn read_raw(
    subkey: &[u16],
    name: &[u16],
    action: &'static str,
) -> Result<Option<(REG_VALUE_TYPE, Vec<u8>)>, StartupError> {
    // The value can change between the size query and the read; a few rounds settle it.
    let mut size = 0u32;
    for _ in 0..4 {
        let mut kind = REG_VALUE_TYPE(0);
        // SAFETY: both pointers describe live locals; a null data pointer asks for the size.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_ANY | RRF_NOEXPAND,
                Some(&mut kind),
                None,
                Some(&mut size),
            )
        };
        if absent(status) {
            return Ok(None);
        }
        if status.0 != 0 {
            return Err(failure(action, status));
        }
        let mut data = vec![0u8; size as usize];
        let mut capacity = size;
        // SAFETY: `data` has exactly `capacity` writable bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_ANY | RRF_NOEXPAND,
                Some(&mut kind),
                Some(data.as_mut_ptr().cast()),
                Some(&mut capacity),
            )
        };
        if status == ERROR_MORE_DATA {
            size = capacity;
            continue;
        }
        if absent(status) {
            return Ok(None);
        }
        if status.0 != 0 {
            return Err(failure(action, status));
        }
        data.truncate(capacity as usize);
        return Ok(Some((kind, data)));
    }
    Err(StartupError::Registry {
        action,
        detail: "the value kept changing".to_owned(),
    })
}

/// Deletes a value; a missing key or value is success.
fn delete_value(subkey: &[u16], name: &[u16], action: &'static str) -> Result<(), StartupError> {
    let mut key = HKEY::default();
    // SAFETY: opens an existing key for value deletion; the handle is closed by `Key`.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut key,
        )
    };
    if absent(status) {
        return Ok(());
    }
    if status.0 != 0 {
        return Err(failure(action, status));
    }
    let key = Key(key);
    // SAFETY: `key` is open with KEY_SET_VALUE; `name` is NUL terminated.
    let status = unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) };
    if status.0 == 0 || absent(status) {
        Ok(())
    } else {
        Err(failure(action, status))
    }
}

/// The per-user `Run` value of this app, and the Windows flag beside it.
pub struct RegistryRunKey {
    run: Vec<u16>,
    approved: Vec<u16>,
    name: Vec<u16>,
}
impl RegistryRunKey {
    pub fn new() -> Self {
        Self::with_keys(RUN_SUBKEY, APPROVED_SUBKEY, VALUE_NAME)
    }
    /// Different keys let the tests run against a temporary location.
    pub(super) fn with_keys(run: &str, approved: &str, name: &str) -> Self {
        Self {
            run: wide_nul(run),
            approved: wide_nul(approved),
            name: wide_nul(name),
        }
    }
}
impl Default for RegistryRunKey {
    fn default() -> Self {
        Self::new()
    }
}

impl RunKey for RegistryRunKey {
    fn read(&mut self) -> Result<RunEntry, StartupError> {
        let command =
            read_raw(&self.run, &self.name, "read the startup entry")?.map(|(kind, data)| {
                if kind != REG_SZ && kind != REG_EXPAND_SZ {
                    // Not text, so not something this app wrote: shows as unrecognised.
                    return String::new();
                }
                text_from_bytes(&data)
            });
        // Windows marks an entry switched off with an odd first byte (03). If this
        // flag cannot be read the entry is treated as on: the Run value is the truth.
        let approved = match read_raw(&self.approved, &self.name, "read the startup flag") {
            Ok(Some((kind, data))) if kind == REG_BINARY => {
                data.first().is_none_or(|flag| flag & 1 == 0)
            }
            _ => true,
        };
        Ok(RunEntry { command, approved })
    }

    fn write(&mut self, command: &str) -> Result<(), StartupError> {
        let action = "turn on startup";
        let mut key = HKEY::default();
        // SAFETY: creates (or opens) the per-user Run key; the handle is closed by `Key`.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(self.run.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPEN_CREATE_OPTIONS(0),
                KEY_SET_VALUE,
                None,
                &mut key,
                None,
            )
        };
        if status.0 != 0 {
            return Err(failure(action, status));
        }
        let key = Key(key);
        let bytes: Vec<u8> = wide_nul(command)
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        // SAFETY: `key` is open with KEY_SET_VALUE; `bytes` is the NUL-terminated UTF-16 text.
        let status = unsafe {
            RegSetValueExW(
                key.0,
                PCWSTR(self.name.as_ptr()),
                None,
                REG_SZ,
                Some(&bytes),
            )
        };
        if status.0 != 0 {
            return Err(failure(action, status));
        }
        // A previous "turned off" flag would keep Windows from running the entry.
        delete_value(&self.approved, &self.name, action)
    }

    fn remove(&mut self) -> Result<(), StartupError> {
        let action = "turn off startup";
        delete_value(&self.run, &self.name, action)?;
        delete_value(&self.approved, &self.name, action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startup::{StartupController, StartupState, command_line};
    use std::path::Path;
    use windows::Win32::System::Registry::{RegDeleteTreeW, RegSetValueExW as set_value};

    /// A throwaway key tree under HKCU, removed when the test ends.
    struct Scratch {
        root: Vec<u16>,
        run: String,
        approved: String,
    }
    impl Scratch {
        fn new(tag: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            let root = format!(
                r"Software\ElevenClock-rs-tests\startup-{tag}-{}-{unique}",
                std::process::id()
            );
            Self {
                run: format!(r"{root}\Run"),
                approved: format!(r"{root}\Approved"),
                root: wide_nul(&root),
            }
        }
        fn key(&self) -> RegistryRunKey {
            RegistryRunKey::with_keys(&self.run, &self.approved, VALUE_NAME)
        }
        /// Put any value in a scratch subkey, the way other programs or Windows would.
        fn put(&self, subkey: &str, name: &str, kind: REG_VALUE_TYPE, bytes: &[u8]) {
            let subkey = wide_nul(subkey);
            let name = wide_nul(name);
            let mut key = HKEY::default();
            // SAFETY: creates a scratch key and sets one value; the handle is closed by `Key`.
            unsafe {
                let status = RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    PCWSTR(subkey.as_ptr()),
                    None,
                    PCWSTR::null(),
                    REG_OPEN_CREATE_OPTIONS(0),
                    KEY_SET_VALUE,
                    None,
                    &mut key,
                    None,
                );
                assert_eq!(status.0, 0);
                let key = Key(key);
                let status = set_value(key.0, PCWSTR(name.as_ptr()), None, kind, Some(bytes));
                assert_eq!(status.0, 0);
            }
        }
        fn put_text(&self, subkey: &str, name: &str, text: &str) {
            let bytes: Vec<u8> = wide_nul(text)
                .iter()
                .flat_map(|unit| unit.to_le_bytes())
                .collect();
            self.put(subkey, name, REG_SZ, &bytes);
        }
        fn text(&self, subkey: &str, name: &str) -> Option<String> {
            read_raw(&wide_nul(subkey), &wide_nul(name), "test")
                .unwrap()
                .map(|(_, data)| text_from_bytes(&data))
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            // SAFETY: deletes only the scratch tree created by this test.
            unsafe {
                let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(self.root.as_ptr()));
            }
        }
    }

    fn exe(path: &str) -> PathBuf {
        PathBuf::from(path)
    }

    #[test]
    fn enable_and_disable_touch_only_the_dedicated_value() {
        let scratch = Scratch::new("isolation");
        // Neighbours: the legacy app's value and an unrelated program's value.
        let legacy = r#""C:\Legacy\ElevenClock.exe""#;
        scratch.put_text(&scratch.run, "elevenClock", legacy);
        scratch.put_text(&scratch.run, "OtherApp", r#""C:\Other\app.exe" /x"#);
        let path = exe(r"C:\Tools\ElevenClock-rs\elevenclock-rs.exe");
        let mut controller = StartupController::new(scratch.key(), path.clone());

        assert_eq!(controller.state().unwrap(), StartupState::Off);
        controller.enable().unwrap();
        assert_eq!(
            scratch.text(&scratch.run, VALUE_NAME).as_deref(),
            Some(command_line(&path).unwrap().as_str())
        );
        // The executable is not on disk, but it is the registered one, so it is On.
        assert_eq!(controller.state().unwrap(), StartupState::On);
        controller.disable().unwrap();
        assert_eq!(controller.state().unwrap(), StartupState::Off);
        controller.disable().unwrap();

        assert_eq!(
            scratch.text(&scratch.run, "elevenClock").as_deref(),
            Some(legacy)
        );
        assert_eq!(
            scratch.text(&scratch.run, "OtherApp").as_deref(),
            Some(r#""C:\Other\app.exe" /x"#)
        );
    }

    #[test]
    fn spaces_and_unicode_survive_the_real_registry() {
        let scratch = Scratch::new("unicode");
        let path = exe(r"C:\Users\Zoë Müller\Apps 日本語\Eleven Clock\elevenclock-rs.exe");
        let mut controller = StartupController::new(scratch.key(), path.clone());
        controller.enable().unwrap();
        let stored = scratch.text(&scratch.run, VALUE_NAME).unwrap();
        assert_eq!(stored, command_line(&path).unwrap());
        assert_eq!(controller.state().unwrap(), StartupState::On);
    }

    #[test]
    fn windows_turning_the_entry_off_is_seen_and_enabling_clears_it() {
        let scratch = Scratch::new("approved");
        let path = exe(r"C:\Tools\ElevenClock-rs\elevenclock-rs.exe");
        let mut controller = StartupController::new(scratch.key(), path);
        controller.enable().unwrap();
        // The 12-byte value Task Manager writes when the user disables an entry.
        let mut disabled = vec![3u8, 0, 0, 0];
        disabled.extend_from_slice(&[0; 8]);
        scratch.put(&scratch.approved, VALUE_NAME, REG_BINARY, &disabled);
        assert_eq!(controller.state().unwrap(), StartupState::DisabledInWindows);
        controller.enable().unwrap();
        assert_eq!(controller.state().unwrap(), StartupState::On);
        // An enabled flag (02) is not "off".
        let mut enabled = vec![2u8, 0, 0, 0];
        enabled.extend_from_slice(&[0; 8]);
        scratch.put(&scratch.approved, VALUE_NAME, REG_BINARY, &enabled);
        assert_eq!(controller.state().unwrap(), StartupState::On);
    }

    #[test]
    fn a_value_of_the_wrong_type_is_reported_not_overwritten_silently() {
        let scratch = Scratch::new("wrong-type");
        scratch.put(&scratch.run, VALUE_NAME, REG_BINARY, &[1, 2, 3]);
        let path = exe(r"C:\Tools\ElevenClock-rs\elevenclock-rs.exe");
        let mut controller = StartupController::new(scratch.key(), path);
        assert!(matches!(
            controller.state().unwrap(),
            StartupState::Stale(crate::startup::Stale::Unrecognised)
        ));
    }

    #[test]
    fn a_moved_copy_is_stale_until_the_user_repairs_it() {
        let scratch = Scratch::new("stale");
        scratch.put_text(
            &scratch.run,
            VALUE_NAME,
            r#""D:\Portable\Gone Folder\elevenclock-rs.exe" --startup"#,
        );
        let path = exe(r"C:\Tools\ElevenClock-rs\elevenclock-rs.exe");
        let mut controller = StartupController::new(scratch.key(), path.clone());
        assert!(matches!(
            controller.state().unwrap(),
            StartupState::Stale(crate::startup::Stale::Missing { .. })
        ));
        // Reading again does not repair it.
        assert!(matches!(
            controller.state().unwrap(),
            StartupState::Stale(_)
        ));
        controller.repair().unwrap();
        assert_eq!(controller.state().unwrap(), StartupState::On);
    }

    #[test]
    fn a_real_existing_file_is_recognised_as_another_copy() {
        let scratch = Scratch::new("other-copy");
        let other = std::env::current_exe().unwrap();
        scratch.put_text(
            &scratch.run,
            VALUE_NAME,
            &command_line(Path::new(&other)).unwrap(),
        );
        let mut controller = StartupController::new(
            scratch.key(),
            exe(r"C:\Tools\ElevenClock-rs\elevenclock-rs.exe"),
        );
        assert!(matches!(
            controller.state().unwrap(),
            StartupState::Stale(crate::startup::Stale::OtherCopy { .. })
        ));
    }

    #[test]
    fn the_running_executable_is_absolute_and_has_no_verbatim_prefix() {
        let path = current_executable().unwrap();
        assert!(path.is_absolute());
        assert!(!path.to_string_lossy().starts_with(r"\\?\"));
        assert!(command_line(&path).is_ok());
    }
}
