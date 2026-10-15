//! Private per-user persistence. No configuration path or file schema is a user API.
use clock_core::{Monitors, Preferences, PreferencesError, Theme, ValidatedPreferences};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const VERSION: u64 = 1;
const MAX_BYTES: u64 = 64 * 1024;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    DamagedData,
    InvalidPreferences(PreferencesError),
    UnsupportedVersion(u64),
    RecoveryRequired,
}
impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => f.write_str("Preferences could not be read or saved. Your previous settings are preserved. Try again."),
            Self::DamagedData => f.write_str("Saved preferences are damaged. Defaults are in use. Settings can preserve the damaged data and save a new selection."),
            Self::InvalidPreferences(e) => write!(f, "Preferences are invalid: {e}"),
            Self::UnsupportedVersion(_) => f.write_str("Saved preferences use an unsupported version. Defaults are in use; saving is disabled. Use a compatible app version."),
            Self::RecoveryRequired => f.write_str("Confirm recovery in Settings before replacing damaged preferences."),
        }
    }
}
impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::InvalidPreferences(e) => Some(e),
            _ => None,
        }
    }
}
impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Internal application state, separate from editable preferences and OS startup state.
#[derive(Clone, Debug)]
pub struct StoredPreferences {
    preferences: ValidatedPreferences,
    first_run_completed: bool,
}
impl Default for StoredPreferences {
    fn default() -> Self {
        Self {
            preferences: Preferences::default()
                .validate()
                .expect("built-in defaults are valid"),
            first_run_completed: false,
        }
    }
}
/// Safe storage boundary. Implementations must leave prior data intact on failed save.
/// Missing storage returns defaults. Recovery preserves the unreadable original.
pub trait PreferencesStore {
    fn load(&self) -> Result<StoredPreferences, StoreError>;
    fn save(&mut self, value: &StoredPreferences) -> Result<(), StoreError>;
    fn preserve_for_recovery(&mut self) -> Result<(), StoreError>;
}
/// Settings owns a draft; only a successfully persisted value becomes active.
pub struct PreferenceSession<S> {
    store: S,
    active: StoredPreferences,
    load_error: Option<StoreError>,
}
impl<S: PreferencesStore> PreferenceSession<S> {
    pub fn open(store: S) -> Self {
        let (active, load_error) = match store.load() {
            Ok(value) => (value, None),
            Err(e) => (StoredPreferences::default(), Some(e)),
        };
        Self {
            store,
            active,
            load_error,
        }
    }
    pub fn active(&self) -> &ValidatedPreferences {
        &self.active.preferences
    }
    pub fn load_error(&self) -> Option<&StoreError> {
        self.load_error.as_ref()
    }
    pub fn first_run_completed(&self) -> bool {
        self.active.first_run_completed
    }
    /// Reset never writes, applies, or changes first-run completion.
    pub fn reset_draft(&self) -> Preferences {
        Preferences::default()
    }
    pub fn apply(&mut self, draft: Preferences) -> Result<(), StoreError> {
        self.commit(draft, self.active.first_run_completed, false)
    }
    /// Explicit Settings recovery preserves the original before saving. Newer versions
    /// cannot be recovered by this app, even through this action.
    pub fn recover(&mut self, draft: Preferences) -> Result<(), StoreError> {
        self.commit(draft, self.active.first_run_completed, true)
    }
    /// Called internally by onboarding after its successful completion.
    pub fn complete_first_run(&mut self) -> Result<(), StoreError> {
        self.commit(self.active.preferences.draft(), true, false)
    }
    fn commit(
        &mut self,
        draft: Preferences,
        first_run_completed: bool,
        recover: bool,
    ) -> Result<(), StoreError> {
        let preferences = draft.validate().map_err(StoreError::InvalidPreferences)?;
        if let Some(StoreError::UnsupportedVersion(v)) = self.load_error {
            return Err(StoreError::UnsupportedVersion(v));
        }
        if self.load_error.is_some() {
            if !recover {
                return Err(StoreError::RecoveryRequired);
            }
            self.store.preserve_for_recovery()?;
        }
        let next = StoredPreferences {
            preferences,
            first_run_completed,
        };
        self.store.save(&next)?;
        self.active = next;
        self.load_error = None;
        Ok(())
    }
}

#[derive(Default)]
pub struct MemoryPreferencesStore {
    value: Option<StoredPreferences>,
}
impl PreferencesStore for MemoryPreferencesStore {
    fn load(&self) -> Result<StoredPreferences, StoreError> {
        Ok(self.value.clone().unwrap_or_default())
    }
    fn save(&mut self, value: &StoredPreferences) -> Result<(), StoreError> {
        self.value = Some(value.clone());
        Ok(())
    }
    fn preserve_for_recovery(&mut self) -> Result<(), StoreError> {
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u64,
    format: String,
    monitors: String,
    font_size_pt: i64,
    theme: String,
    first_run_completed: bool,
}
fn decode(bytes: &[u8]) -> Result<StoredPreferences, StoreError> {
    // Read the version before decoding this version's fields, including when a newer
    // writer changed the entire payload shape. Never rewrite an unsupported version.
    let raw: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| StoreError::DamagedData)?;
    let version = raw
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or(StoreError::DamagedData)?;
    if version != VERSION {
        return Err(StoreError::UnsupportedVersion(version));
    }
    let doc: Document = serde_json::from_slice(bytes).map_err(|_| StoreError::DamagedData)?;
    let preferences = (|| {
        Preferences {
            format: doc.format,
            monitors: Monitors::try_from(doc.monitors.as_str())?,
            font_size_pt: u8::try_from(doc.font_size_pt)
                .map_err(|_| PreferencesError::FontSizeOutOfRange(doc.font_size_pt))?,
            theme: Theme::try_from(doc.theme.as_str())?,
        }
        .validate()
    })()
    .map_err(StoreError::InvalidPreferences)?;
    Ok(StoredPreferences {
        preferences,
        first_run_completed: doc.first_run_completed,
    })
}
fn encode(value: &StoredPreferences) -> Result<Vec<u8>, StoreError> {
    let p = &value.preferences;
    serde_json::to_vec(&Document {
        version: VERSION,
        format: p.format_source().into(),
        monitors: match p.monitors() {
            Monitors::Primary => "primary",
            Monitors::All => "all",
        }
        .into(),
        font_size_pt: i64::from(p.font_size_pt()),
        theme: match p.theme() {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
        .into(),
        first_run_completed: value.first_run_completed,
    })
    .map_err(|_| StoreError::DamagedData)
}

/// Production constructor uses Known Folders, never HOME or a path override.
pub struct LocalPreferencesStore {
    path: PathBuf,
    recovery_save_allowed: bool,
}
#[cfg(windows)]
impl LocalPreferencesStore {
    pub fn new() -> Result<Self, StoreError> {
        use windows::Win32::{
            System::Com::CoTaskMemFree,
            UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
        };
        // SAFETY: API allocates a NUL-terminated path. Copy it before freeing with
        // CoTaskMemFree on every path; no native pointer escapes this function.
        let folder = unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None) }
            .map_err(|e| StoreError::Io(io::Error::other(e)))?;
        let wide = unsafe { folder.as_wide() };
        use std::os::windows::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_wide(wide));
        unsafe { CoTaskMemFree(Some(folder.0.cast())) };
        Ok(Self {
            path: path.join("Mainspring").join("preferences.dat"),
            recovery_save_allowed: false,
        })
    }
}
impl PreferencesStore for LocalPreferencesStore {
    fn load(&self) -> Result<StoredPreferences, StoreError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(StoredPreferences::default());
            }
            Err(e) => return Err(e.into()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(StoreError::DamagedData);
        }
        decode(&bytes)
    }
    fn save(&mut self, value: &StoredPreferences) -> Result<(), StoreError> {
        // Also guard direct trait callers, rather than relying only on a session.
        match self.load() {
            Err(StoreError::UnsupportedVersion(v)) => {
                return Err(StoreError::UnsupportedVersion(v));
            }
            Err(StoreError::DamagedData | StoreError::InvalidPreferences(_))
                if self.recovery_save_allowed => {}
            Err(e) => return Err(e),
            Ok(_) => (),
        }
        self.recovery_save_allowed = false;
        self.write(value, || Ok(()))
    }
    fn preserve_for_recovery(&mut self) -> Result<(), StoreError> {
        // Copy first; the original remains in place until a successful atomic save.
        // Recovery permission is private to this store and scoped to the next save.
        if let Err(StoreError::UnsupportedVersion(v)) = self.load() {
            return Err(StoreError::UnsupportedVersion(v));
        }
        let mut source = File::open(&self.path)?;
        let (path, mut backup) = unique_file(&self.path, "preserved")?;
        let result = (|| {
            io::copy(&mut source, &mut backup)?;
            backup.sync_all()
        })();
        drop(backup);
        if let Err(e) = result {
            let _ = fs::remove_file(path);
            return Err(e.into());
        }
        self.recovery_save_allowed = true;
        Ok(())
    }
}

struct PendingFile(PathBuf);
impl Drop for PendingFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn unique_file(target: &Path, suffix: &str) -> io::Result<(PathBuf, File)> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::other("missing store directory"))?;
    for _ in 0..100 {
        let n = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("preferences.{}.{n}.{suffix}", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other(
        "could not allocate temporary preference file",
    ))
}
impl LocalPreferencesStore {
    fn write(
        &self,
        value: &StoredPreferences,
        before_replace: impl FnOnce() -> io::Result<()>,
    ) -> Result<(), StoreError> {
        let bytes = encode(value)?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("missing store directory"))?;
        fs::create_dir_all(parent)?;
        let (path, mut file) = unique_file(&self.path, "tmp")?;
        let pending = PendingFile(path);
        let written = (|| {
            file.write_all(&bytes)?;
            file.sync_all()
        })();
        // Close before Windows move/replacement; no handle remains open at commit.
        drop(file);
        written?;
        before_replace()?;
        replace(&pending.0, &self.path)?;
        Ok(())
    }
    #[cfg(test)]
    fn temporary(path: PathBuf) -> Self {
        Self {
            path,
            recovery_save_allowed: false,
        }
    }
}
#[cfg(windows)]
fn replace(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        core::PCWSTR,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: owned NUL-terminated buffers live for the synchronous API call.
    // Same directory/volume. MoveFileExW supports both first create and replacement;
    // locked/denied destinations fail without truncating the previous file.
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(io::Error::other)
}
#[cfg(not(windows))]
fn replace(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temporary() -> (tempfile::TempDir, LocalPreferencesStore) {
        let dir = tempfile::tempdir().unwrap();
        let store =
            LocalPreferencesStore::temporary(dir.path().join("owned").join("preferences.dat"));
        (dir, store)
    }
    fn changed() -> Preferences {
        Preferences {
            theme: Theme::Dark,
            monitors: Monitors::All,
            font_size_pt: 24,
            ..Preferences::default()
        }
    }
    #[test]
    fn first_save_replacement_and_first_run() {
        let (_dir, store) = temporary();
        let path = store.path.clone();
        let mut session = PreferenceSession::open(store);
        assert!(session.load_error().is_none());
        assert!(!path.exists());
        assert!(!session.first_run_completed());
        session.apply(Preferences::default()).unwrap();
        assert_eq!(
            decode(&fs::read(&path).unwrap())
                .unwrap()
                .preferences
                .draft(),
            Preferences::default()
        );
        session.apply(changed()).unwrap();
        session.complete_first_run().unwrap();
        let session = PreferenceSession::open(LocalPreferencesStore::temporary(path));
        assert_eq!(session.active().draft(), changed());
        assert!(session.first_run_completed());
        assert_eq!(session.reset_draft(), Preferences::default());
        assert_eq!(session.active().draft(), changed());
    }
    #[test]
    fn invalid_values_have_specific_errors() {
        let bytes = encode(&StoredPreferences::default()).unwrap();
        let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for (field, value, expected) in [
            (
                "monitors",
                serde_json::json!("other"),
                PreferencesError::InvalidMonitors("other".into()),
            ),
            (
                "theme",
                serde_json::json!("other"),
                PreferencesError::InvalidTheme("other".into()),
            ),
            (
                "font_size_pt",
                serde_json::json!(-1),
                PreferencesError::FontSizeOutOfRange(-1),
            ),
            (
                "font_size_pt",
                serde_json::json!(256),
                PreferencesError::FontSizeOutOfRange(256),
            ),
            (
                "font_size_pt",
                serde_json::json!(5),
                PreferencesError::FontSizeOutOfRange(5),
            ),
            (
                "font_size_pt",
                serde_json::json!(25),
                PreferencesError::FontSizeOutOfRange(25),
            ),
        ] {
            let mut raw = original.clone();
            raw[field] = value;
            assert!(
                matches!(decode(&serde_json::to_vec(&raw).unwrap()), Err(StoreError::InvalidPreferences(e)) if e == expected)
            );
        }
        let mut raw = original;
        raw["format"] = serde_json::json!("%n");
        assert!(matches!(
            decode(&serde_json::to_vec(&raw).unwrap()),
            Err(StoreError::InvalidPreferences(PreferencesError::Format(_)))
        ));
        for version in [0, 2, u64::MAX] {
            assert!(
                matches!(decode(format!("{{\"version\":{version},\"future\":true}}").as_bytes()), Err(StoreError::UnsupportedVersion(v)) if v == version)
            );
        }
    }
    #[test]
    fn damaged_data_is_preserved_and_recovery_is_explicit() {
        let (_dir, store) = temporary();
        fs::create_dir_all(store.path.parent().unwrap()).unwrap();
        fs::write(&store.path, b"broken original").unwrap();
        let path = store.path.clone();
        let mut session = PreferenceSession::open(store);
        assert!(matches!(
            session.load_error(),
            Some(StoreError::DamagedData)
        ));
        assert_eq!(session.active().draft(), Preferences::default());
        assert!(matches!(
            session.apply(changed()),
            Err(StoreError::RecoveryRequired)
        ));
        assert_eq!(fs::read(&path).unwrap(), b"broken original");
        session.recover(changed()).unwrap();
        assert!(session.load_error().is_none());
        let backups: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|s| s == "preserved"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(backups[0].path()).unwrap(), b"broken original");
        assert_eq!(session.active().draft(), changed());
    }
    #[test]
    fn unsupported_version_cannot_be_overwritten() {
        let (_dir, mut store) = temporary();
        fs::create_dir_all(store.path.parent().unwrap()).unwrap();
        let bytes = b"{\"version\":2,\"new_shape\":true}";
        fs::write(&store.path, bytes).unwrap();
        assert!(matches!(
            store.save(&StoredPreferences::default()),
            Err(StoreError::UnsupportedVersion(2))
        ));
        let path = store.path.clone();
        let mut session = PreferenceSession::open(store);
        assert!(matches!(
            session.recover(changed()),
            Err(StoreError::UnsupportedVersion(2))
        ));
        assert!(matches!(
            session.complete_first_run(),
            Err(StoreError::UnsupportedVersion(2))
        ));
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn injected_failure_before_replace_preserves_disk_and_cleans_temp() {
        let (_dir, mut store) = temporary();
        store.save(&StoredPreferences::default()).unwrap();
        let before = fs::read(&store.path).unwrap();
        let next = StoredPreferences {
            preferences: changed().validate().unwrap(),
            first_run_completed: true,
        };
        assert!(
            store
                .write(&next, || Err(io::Error::other("injected")))
                .is_err()
        );
        assert_eq!(fs::read(&store.path).unwrap(), before);
        assert_eq!(
            fs::read_dir(store.path.parent().unwrap()).unwrap().count(),
            1
        );
    }
    struct FailingStore(MemoryPreferencesStore);
    impl PreferencesStore for FailingStore {
        fn load(&self) -> Result<StoredPreferences, StoreError> {
            self.0.load()
        }
        fn save(&mut self, _: &StoredPreferences) -> Result<(), StoreError> {
            Err(io::Error::other("injected").into())
        }
        fn preserve_for_recovery(&mut self) -> Result<(), StoreError> {
            Ok(())
        }
    }
    #[test]
    fn failed_save_keeps_active_and_first_run() {
        let mut session = PreferenceSession::open(FailingStore(MemoryPreferencesStore::default()));
        assert!(session.apply(changed()).is_err());
        assert!(session.complete_first_run().is_err());
        assert_eq!(session.active().draft(), Preferences::default());
        assert!(!session.first_run_completed());
    }
    #[test]
    fn memory_store_and_reset_draft() {
        let mut session = PreferenceSession::open(MemoryPreferencesStore::default());
        session.apply(changed()).unwrap();
        session.complete_first_run().unwrap();
        let draft = session.reset_draft();
        assert_eq!(session.active().draft(), changed());
        session.apply(draft).unwrap();
        assert!(session.first_run_completed());
        let reopened = PreferenceSession::open(session.store);
        assert_eq!(reopened.active().draft(), Preferences::default());
        assert!(reopened.first_run_completed());
    }
    #[test]
    fn oversized_and_missing_fields_are_damaged() {
        assert!(matches!(decode(b"{}"), Err(StoreError::DamagedData)));
        let (_dir, store) = temporary();
        fs::create_dir_all(store.path.parent().unwrap()).unwrap();
        fs::write(&store.path, vec![b' '; MAX_BYTES as usize + 1]).unwrap();
        assert!(matches!(store.load(), Err(StoreError::DamagedData)));
    }
    #[test]
    fn failed_first_create_and_legacy_files_remain_untouched() {
        let (dir, store) = temporary();
        let legacy = dir.path().join(".elevenclock");
        fs::create_dir(&legacy).unwrap();
        fs::write(legacy.join("legacy-setting"), b"keep").unwrap();
        assert!(
            store
                .write(&StoredPreferences::default(), || Err(io::Error::other(
                    "injected"
                )))
                .is_err()
        );
        assert!(!store.path.exists());
        assert_eq!(
            fs::read_dir(store.path.parent().unwrap()).unwrap().count(),
            0
        );
        let mut session = PreferenceSession::open(store);
        session.apply(changed()).unwrap();
        session.complete_first_run().unwrap();
        session.apply(session.reset_draft()).unwrap();
        assert_eq!(fs::read(legacy.join("legacy-setting")).unwrap(), b"keep");
        assert_eq!(fs::read_dir(legacy).unwrap().count(), 1);
    }
    #[cfg(windows)]
    #[test]
    fn known_folder_constructor_is_app_owned() {
        let store = LocalPreferencesStore::new().unwrap();
        assert!(store.path.is_absolute());
        assert!(
            store
                .path
                .ends_with(Path::new("Mainspring").join("preferences.dat"))
        );
    }
    #[cfg(windows)]
    #[test]
    fn locked_target_preserves_disk_and_active() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_dir, store) = temporary();
        let path = store.path.clone();
        let mut session = PreferenceSession::open(store);
        session.apply(Preferences::default()).unwrap();
        let before = fs::read(&path).unwrap();
        // Allow reads/writes, deny delete sharing required by MoveFileExW.
        let lock = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();
        assert!(session.apply(changed()).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(session.active().draft(), Preferences::default());
        drop(lock);
        session.apply(changed()).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn read_only_target_preserves_disk_and_active() {
        let (_dir, store) = temporary();
        let path = store.path.clone();
        let mut session = PreferenceSession::open(store);
        session.apply(Preferences::default()).unwrap();
        let before = fs::read(&path).unwrap();
        let original_permissions = fs::metadata(&path).unwrap().permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions.clone()).unwrap();
        let result = session.apply(changed());
        fs::set_permissions(&path, original_permissions).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(session.active().draft(), Preferences::default());
    }
}
