//! Development overlay experiment. Not integrated into the installed app.
use clock_core::PhysicalRect;

/// Unknown/changed shell state always removes coverage. Fullscreen is evaluated
/// per monitor; partially revealed auto-hide taskbars never get an overlay.
pub fn may_show(
    lease_current: bool,
    taskbar_visible: bool,
    measured: PhysicalRect,
    expected: PhysicalRect,
    monitor: PhysicalRect,
    foreground: Option<PhysicalRect>,
    foreground_is_shell: bool,
) -> bool {
    lease_current
        && taskbar_visible
        && measured == expected
        && monitor.contains(measured)
        && (foreground_is_shell || !foreground.is_some_and(|r| r.contains(monitor)))
}

#[cfg(all(windows, feature = "overlay-spike"))]
mod native;
#[cfg(all(windows, feature = "overlay-spike"))]
pub use native::{ExperimentSlot, Update, run};

#[cfg(all(windows, feature = "overlay-spike"))]
pub(crate) use native::check_slot;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hides_on_fullscreen_taskbar_slide_or_lost_lease() {
        let monitor = PhysicalRect::new(-1920, 0, 1920, 1080).unwrap();
        let bar = PhysicalRect::new(-1920, 1032, 1920, 48).unwrap();
        let slide = PhysicalRect::new(-1920, 1078, 1920, 48).unwrap();
        assert!(may_show(true, true, bar, bar, monitor, None, false));
        assert!(!may_show(false, true, bar, bar, monitor, None, false));
        assert!(!may_show(true, false, bar, bar, monitor, None, false));
        assert!(!may_show(true, true, slide, bar, monitor, None, false));
        assert!(!may_show(
            true,
            true,
            bar,
            bar,
            monitor,
            Some(monitor),
            false
        ));
        assert!(may_show(true, true, bar, bar, monitor, Some(monitor), true));
        let other = PhysicalRect::new(0, 0, 1920, 1080).unwrap();
        assert!(may_show(true, true, bar, bar, monitor, Some(other), false));
    }
}
