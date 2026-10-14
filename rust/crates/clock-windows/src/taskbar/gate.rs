//! The support gate for taskbar clock rectangles (R02 / R03 / #21).
//!
//! The project is a work in progress with no released build, so the gate is open:
//! the runtime uses the R02 *experimental* UIA candidate (after geometric
//! validation) to place the clock overlay. P1 is still NO-GO (#23: overlay coverage
//! after an Explorer restart is not reliable), so expect rough edges.
//!
//! `Gate::Closed` is kept as the fail-safe mode: with it the parent never accepts a
//! clock rectangle whatever the helper claims, so the reducer creates no overlay and
//! the native clock is untouched. Tests exercise it, and a release can switch
//! [`Gate::current`] back to it.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Gate {
    /// Clock rectangles are never produced (fail-safe).
    Closed,
    /// The R02 experimental UIA candidate may supply a rectangle after geometric
    /// validation. Not a supported selector.
    Experimental,
}
impl Gate {
    pub const fn current() -> Self {
        Self::Experimental
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
    fn the_work_in_progress_build_uses_the_experimental_candidate() {
        assert_eq!(Gate::current(), Gate::Experimental);
        assert!(Gate::current().needs_uia());
        assert!(!Gate::Closed.needs_uia());
    }
}
