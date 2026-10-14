#![forbid(unsafe_code)]
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod app;

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    app::main()
}

// Allows host workspace checks; Linux builds do not implement a user app.
#[cfg(not(windows))]
fn main() {}
