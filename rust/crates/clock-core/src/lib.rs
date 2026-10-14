//! Platform-independent value contracts. No native handles or filesystem policy.
#![forbid(unsafe_code)]

pub mod error;
pub mod model;
pub use error::GeometryError;
pub use model::*;
