use super::{HOST_CLASS, InstanceNames, WM_SHOW_SETTINGS};
use std::time::{Duration, Instant};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HLOCAL, LPARAM, LocalFree,
            WPARAM,
        },
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken},
        UI::WindowsAndMessaging::{
            AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW,
        },
    },
    core::{HSTRING, PWSTR, Result},
};

/// Owns the per-user mutex for the life of the primary process. The mutex is
/// never "owned" for synchronization; only its existence matters, and the kernel
/// releases it when the process ends on any path, so no stale lock can remain.
pub struct InstanceGuard {
    mutex: HANDLE,
    names: InstanceNames,
}
impl InstanceGuard {
    pub fn names(&self) -> &InstanceNames {
        &self.names
    }
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // SAFETY: the handle was created by CreateMutexW and is closed once.
        let _ = unsafe { CloseHandle(self.mutex) };
    }
}

pub enum Acquired {
    Primary(InstanceGuard),
    Existing(InstanceNames),
}

fn current_user_sid() -> Result<String> {
    // SAFETY: standard two-call token query. The token handle is closed on every
    // path, the buffer is sized by the first call, and the converted SID string
    // is LocalFree'd after copying.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let result = (|| {
            let mut needed = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
            if needed == 0 {
                return Err(windows::core::Error::from_win32());
            }
            // u64 elements keep TOKEN_USER (pointer-aligned) correctly aligned.
            let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
            GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                needed,
                &mut needed,
            )?;
            let user = &*(buffer.as_ptr().cast::<TOKEN_USER>());
            let mut text = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut text)?;
            let sid = text
                .to_string()
                .map_err(|_| windows::core::Error::from_win32());
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            sid
        })();
        let _ = CloseHandle(token);
        result
    }
}

/// Decide ownership. The first process for this user and session becomes
/// primary; later ones are `Existing` and should call [`signal_existing`].
pub fn acquire() -> Result<Acquired> {
    let names = InstanceNames::for_user(&current_user_sid()?);
    // SAFETY: the name is NUL terminated by HSTRING; default security makes the
    // object accessible to this user only. No initial ownership is requested.
    let (mutex, existed) = unsafe {
        let mutex = CreateMutexW(None, false, &HSTRING::from(names.mutex.as_str()))?;
        (mutex, GetLastError() == ERROR_ALREADY_EXISTS)
    };
    if existed {
        // SAFETY: closing our duplicate reference to the existing object.
        let _ = unsafe { CloseHandle(mutex) };
        Ok(Acquired::Existing(names))
    } else {
        Ok(Acquired::Primary(InstanceGuard { mutex, names }))
    }
}

/// Ask the running instance to open or focus Settings. The primary may still be
/// creating its host window (simultaneous launches) or shutting down, so the
/// lookup is bounded rather than assumed. Returns whether the signal was posted.
pub fn signal_existing(names: &InstanceNames, wait: Duration) -> bool {
    let title = HSTRING::from(names.window_title.as_str());
    let class = HSTRING::from(HOST_CLASS);
    let started = Instant::now();
    loop {
        // SAFETY: lookups and a post to a window of another process of the same
        // user; a stale handle fails safely. Posting carries no pointers.
        unsafe {
            if let Ok(host) = FindWindowW(&class, &title)
                && !host.is_invalid()
            {
                let mut pid = 0;
                GetWindowThreadProcessId(host, Some(&mut pid));
                // Let the existing process take the foreground for its window.
                let _ = AllowSetForegroundWindow(pid);
                return PostMessageW(Some(host), WM_SHOW_SETTINGS, WPARAM(0), LPARAM(0)).is_ok();
            }
        }
        if started.elapsed() >= wait {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
