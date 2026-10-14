//! The production support gate for taskbar clock rectangles (R02 / R03 / #21).
//!
//! P1 is NO-GO: Explorer-restart coverage recovery failed on every hosted profile
//! and the stock Windows 11 x64 / mixed-DPI evidence is missing. Until an
//! evidence-backed GO is recorded, the runtime performs discovery and reconciles
//! every event, but no snapshot ever carries a clock rectangle, so the reducer
//! creates no overlay and the native clock stays untouched.
//!
//! Opening the gate is deliberately a compile-time development decision: the
//! `unverified-overlays` feature exists for hosted/desktop experiments and must
//! never be enabled for a release build.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Gate {
    /// Production behaviour: clock rectangles are never produced.
    Closed,
    /// Development builds only: the R02 *experimental* UIA candidate may supply a
    /// rectangle after geometric validation. Not a supported selector.
    Experimental,
}
impl Gate {
    pub const fn current() -> Self {
        if cfg!(feature = "unverified-overlays") {
            Self::Experimental
        } else {
            Self::Closed
        }
    }
    /// Whether discovery needs the (expensive, blocking-provider) UIA inventory.
    pub const fn needs_uia(self) -> bool {
        matches!(self, Self::Experimental)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_default_build_keeps_the_gate_closed() {
        if !cfg!(feature = "unverified-overlays") {
            assert_eq!(Gate::current(), Gate::Closed);
            assert!(!Gate::current().needs_uia());
        } else {
            assert_eq!(Gate::current(), Gate::Experimental);
        }
    }
}
