//! Windows side of Support: the OS build and the clipboard.
use super::{APP_COMMIT, Clipboard, ClipboardError, ClipboardFault, Environment};
use std::{thread, time::Duration};
use windows::{
    Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::CF_UNICODETEXT,
            Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW},
            SystemInformation::{
                GetNativeSystemInfo, PROCESSOR_ARCHITECTURE_AMD64, PROCESSOR_ARCHITECTURE_ARM64,
                PROCESSOR_ARCHITECTURE_INTEL, SYSTEM_INFO,
            },
        },
    },
    core::{PCWSTR, w},
};

/// Another program may hold the clipboard for a moment. Wait a little, never long:
/// this runs on the UI thread, so the whole wait is capped at about 40 ms.
const OPEN_ATTEMPTS: u32 = 5;
const OPEN_PAUSE: Duration = Duration::from_millis(10);

/// Writes Unicode text to the Windows clipboard. `owner` must be a live window of
/// this thread: with no owner Windows refuses the data after clearing the clipboard.
pub struct WindowsClipboard {
    owner: HWND,
}
impl WindowsClipboard {
    pub fn new(owner: HWND) -> Self {
        Self { owner }
    }
}

/// Memory Windows takes over only when `SetClipboardData` succeeds.
struct Prepared(HGLOBAL);
impl Prepared {
    fn new(text: &str) -> Result<Self, ClipboardError> {
        let failed = ClipboardError {
            fault: ClipboardFault::OutOfMemory,
            replaced: false,
        };
        let units: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let bytes = units.len() * size_of::<u16>();
        // SAFETY: a fresh movable block of exactly `bytes`, locked only while the
        // copy runs, then unlocked; on any failure the block is freed here.
        unsafe {
            let handle = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|_| failed)?;
            let target = GlobalLock(handle).cast::<u16>();
            if target.is_null() {
                let _ = GlobalFree(Some(handle));
                return Err(failed);
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
            let _ = GlobalUnlock(handle);
            Ok(Self(handle))
        }
    }
}

impl Clipboard for WindowsClipboard {
    /// Everything that can fail without touching the clipboard (memory, waiting for
    /// access) happens before the old contents are cleared, so those failures leave
    /// the clipboard exactly as it was.
    fn set_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        let prepared = Prepared::new(text)?;
        let mut opened = false;
        for attempt in 0..OPEN_ATTEMPTS {
            // SAFETY: opens the clipboard for our live window; paired with the
            // CloseClipboard below on this same thread.
            if unsafe { OpenClipboard(Some(self.owner)) }.is_ok() {
                opened = true;
                break;
            }
            if attempt + 1 < OPEN_ATTEMPTS {
                thread::sleep(OPEN_PAUSE);
            }
        }
        if !opened {
            // SAFETY: Windows never received the block, so it is still ours.
            unsafe {
                let _ = GlobalFree(Some(prepared.0));
            }
            return Err(ClipboardError {
                fault: ClipboardFault::Busy,
                replaced: false,
            });
        }
        // SAFETY: the clipboard is open on this thread. A successful
        // SetClipboardData takes ownership of the block; otherwise it is freed.
        let result = unsafe {
            if EmptyClipboard().is_err() {
                let _ = GlobalFree(Some(prepared.0));
                Err(ClipboardError {
                    fault: ClipboardFault::Rejected,
                    replaced: false,
                })
            } else if SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(prepared.0.0)))
                .is_err()
            {
                let _ = GlobalFree(Some(prepared.0));
                Err(ClipboardError {
                    fault: ClipboardFault::Rejected,
                    replaced: true,
                })
            } else {
                Ok(())
            }
        };
        // SAFETY: closes what this call opened.
        unsafe {
            let _ = CloseClipboard();
        }
        result
    }
}

const VERSION_KEY: PCWSTR = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");

fn registry_text(name: PCWSTR) -> Option<String> {
    let mut buffer = [0u16; 64];
    let mut bytes = (buffer.len() * size_of::<u16>()) as u32;
    // SAFETY: the buffer and its byte size describe the same writable memory.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            VERSION_KEY,
            name,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status.0 != 0 {
        return None;
    }
    let length = (bytes as usize / size_of::<u16>()).min(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..length]);
    let text = text.trim_end_matches('\0').trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn registry_number(name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut bytes = size_of::<u32>() as u32;
    // SAFETY: a DWORD-sized output for a DWORD-only query.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            VERSION_KEY,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::from_mut(&mut value).cast()),
            Some(&mut bytes),
        )
    };
    (status.0 == 0).then_some(value)
}

/// `10.0.26200.1234 (25H2)`: Windows 10 and 11 both report 10.0; the build number
/// tells them apart.
pub fn windows_text(build: Option<&str>, revision: Option<u32>, release: Option<&str>) -> String {
    let Some(build) = build else {
        return "unknown".to_owned();
    };
    let mut text = format!("10.0.{build}");
    if let Some(revision) = revision {
        text.push_str(&format!(".{revision}"));
    }
    if let Some(release) = release {
        text.push_str(&format!(" ({release})"));
    }
    text
}

fn os_architecture() -> &'static str {
    let mut info = SYSTEM_INFO::default();
    // SAFETY: fills a caller-owned structure; reports the real OS even for an
    // emulated (x64 on ARM64) process.
    unsafe { GetNativeSystemInfo(&mut info) };
    // SAFETY: the architecture field is valid in every variant of the union.
    let architecture = unsafe { info.Anonymous.Anonymous.wProcessorArchitecture };
    if architecture == PROCESSOR_ARCHITECTURE_AMD64 {
        "x64"
    } else if architecture == PROCESSOR_ARCHITECTURE_ARM64 {
        "ARM64"
    } else if architecture == PROCESSOR_ARCHITECTURE_INTEL {
        "x86"
    } else {
        "unknown"
    }
}

fn app_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "ARM64",
        "x86" => "x86",
        other => other,
    }
}

pub fn detect_environment() -> Environment {
    Environment {
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        commit: APP_COMMIT.to_owned(),
        windows: windows_text(
            registry_text(w!("CurrentBuildNumber")).as_deref(),
            registry_number(w!("UBR")),
            registry_text(w!("DisplayVersion")).as_deref(),
        ),
        os_architecture: os_architecture().to_owned(),
        app_architecture: app_architecture().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_text_names_build_revision_and_release() {
        assert_eq!(
            windows_text(Some("26200"), Some(6899), Some("25H2")),
            "10.0.26200.6899 (25H2)"
        );
        assert_eq!(windows_text(Some("22631"), None, None), "10.0.22631");
        assert_eq!(windows_text(None, Some(1), Some("x")), "unknown");
    }

    #[test]
    fn this_machine_reports_a_build_and_architectures() {
        let environment = detect_environment();
        assert!(
            environment.windows.starts_with("10.0."),
            "{}",
            environment.windows
        );
        assert_ne!(environment.os_architecture, "unknown");
        assert_ne!(environment.app_architecture, "");
        assert!(!environment.commit.is_empty());
    }
}
