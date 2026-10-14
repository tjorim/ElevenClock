//! Windows-only safe facade. All native resources remain on the UI thread.
#![cfg(windows)]

mod settings;
pub use settings::{PlatformError, run_settings};
