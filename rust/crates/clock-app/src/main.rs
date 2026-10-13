#![forbid(unsafe_code)]
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    match clock_windows::run_settings() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            error.show();
            std::process::ExitCode::FAILURE
        }
    }
}

// Allows host workspace checks; Linux builds do not implement a user app.
#[cfg(not(windows))]
fn main() {}
