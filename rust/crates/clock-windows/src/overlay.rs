//! Development overlay experiment. Not integrated into the installed app.
use clock_core::PhysicalRect;

/// Sample strictly inside the measured clock bounds. After a fractional DPI
/// transition UIA can assign a shared integer boundary to the adjacent button
/// even though its bounding rectangle ends there. Geometry/exclusion validation
/// still uses the full rectangle; point hits supplement that validation.
#[cfg(any(test, all(windows, feature = "overlay-spike")))]
pub(crate) fn clock_hit_points(clock: PhysicalRect) -> Option<impl Iterator<Item = (i32, i32)>> {
    if !(2..=512).contains(&clock.width()) || !(2..=256).contains(&clock.height()) {
        return None;
    }
    let xs = (clock.left() + 1..clock.right())
        .step_by(4)
        .chain([clock.right() - 1]);
    let ys = (clock.top() + 1..clock.bottom())
        .step_by(4)
        .chain([clock.bottom() - 1]);
    Some(ys.flat_map(move |y| xs.clone().map(move |x| (x, y))))
}

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

/// Mirror only the visible taskbar's band. Hidden coverage keeps its band: a
/// hidden window covers nothing, and Windows silently ignores `HWND_TOPMOST`
/// from a background process while the desktop is foreground (#23), so a
/// demoted overlay could not get back above the taskbar. Overlays are created
/// in the taskbar's band instead of being promoted later.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BandAction {
    Preserve,
    PromoteToTaskbar,
    Demote,
}
pub fn band_action(show: bool, taskbar_topmost: bool, overlay_topmost: bool) -> BandAction {
    match (show, taskbar_topmost, overlay_topmost) {
        (true, true, false) => BandAction::PromoteToTaskbar,
        (true, false, true) => BandAction::Demote,
        _ => BandAction::Preserve,
    }
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
    fn restored_dpi_samples_avoid_the_shared_neighbor_boundary() {
        let clock = PhysicalRect::new(910, 720, 74, 48).unwrap();
        let points: Vec<_> = clock_hit_points(clock).unwrap().collect();
        assert_eq!(points.first(), Some(&(911, 721)));
        assert!(points.contains(&(983, 767)));
        assert!(
            points
                .iter()
                .all(|&(x, y)| x > 910 && x < 984 && y > 720 && y < 768)
        );
        for axis in [
            points
                .iter()
                .map(|p| p.0)
                .collect::<std::collections::BTreeSet<_>>(),
            points.iter().map(|p| p.1).collect(),
        ] {
            let values: Vec<_> = axis.into_iter().collect();
            assert!(values.windows(2).all(|pair| pair[1] - pair[0] <= 4));
        }
    }

    #[test]
    fn interior_sampling_handles_negative_monitors_and_rejects_unprovable_sizes() {
        let clock = PhysicalRect::new(-1920, -48, 2, 2).unwrap();
        assert!(
            clock_hit_points(clock)
                .unwrap()
                .all(|point| point == (-1919, -47))
        );
        for (width, height) in [(1, 48), (74, 1), (513, 48), (74, 257)] {
            assert!(clock_hit_points(PhysicalRect::new(0, 0, width, height).unwrap()).is_none());
        }
    }
    #[test]
    fn band_changes_only_while_shown() {
        for taskbar_topmost in [false, true] {
            for overlay_topmost in [false, true] {
                assert_eq!(
                    band_action(false, taskbar_topmost, overlay_topmost),
                    BandAction::Preserve
                );
            }
        }
        assert_eq!(band_action(true, true, false), BandAction::PromoteToTaskbar);
        assert_eq!(band_action(true, true, true), BandAction::Preserve);
        assert_eq!(band_action(true, false, true), BandAction::Demote);
        assert_eq!(band_action(true, false, false), BandAction::Preserve);
    }
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
