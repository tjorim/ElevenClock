//! Safe platform facade. Native resources stay on their owning thread/process.

#[cfg(windows)]
mod settings;
pub mod taskbar;
#[cfg(windows)]
pub use settings::{PlatformError, run_settings};

pub mod overlay;
#[cfg(windows)]
pub mod prototype_interaction;
