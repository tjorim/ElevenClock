use std::fmt;

/// A rectangle must have positive size and representable signed edges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryError {
    NonPositiveSize,
    EdgeOverflow,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NonPositiveSize => "rectangle width and height must be positive",
            Self::EdgeOverflow => "rectangle edge exceeds signed physical coordinates",
        })
    }
}
impl std::error::Error for GeometryError {}
