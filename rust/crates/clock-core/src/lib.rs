//! Platform-independent value contracts. No native handles or filesystem policy.
#![forbid(unsafe_code)]

pub mod error;
pub mod format;
pub use format::{CompiledFormat, FormatError, FormatErrorKind};
pub mod model;
pub use error::GeometryError;
pub use model::*;

pub mod preferences;
pub use preferences::{Monitors, Preferences, PreferencesError, Theme, ValidatedPreferences};

pub mod schedule;
pub mod state;
pub use schedule::next_delay;
pub use state::{State, reduce};
