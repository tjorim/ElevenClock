//! Pure interpretation of what the shell currently shows (auto-hide, fullscreen).
//! Native code gathers an [`Observation`]; this module decides what it means.
use clock_core::{PhysicalRect, TaskbarSnapshot, TaskbarVisibility};

/// A taskbar's present state as read from the OS, without side effects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Observation {
    /// The taskbar window still exists.
    pub exists: bool,
    pub visible: bool,
    pub rect: Option<PhysicalRect>,
    pub dpi: u32,
    pub monitor_rect: Option<PhysicalRect>,
    /// A foreground window covers this taskbar's whole monitor.
    pub fullscreen: bool,
}
impl Observation {
    pub const GONE: Self = Self {
        exists: false,
        visible: false,
        rect: None,
        dpi: 0,
        monitor_rect: None,
        fullscreen: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Placement {
    /// The window or its monitor disappeared: the snapshot cannot be trusted.
    Gone,
    /// It is somewhere the snapshot does not describe: rediscover.
    Relocated,
    /// It still occupies the discovered slot, or is auto-hidden/sliding.
    Present(TaskbarVisibility),
}

/// Auto-hide parks the window partly outside its monitor, so any rectangle that
/// is not the discovered one and is not fully on the monitor is "hidden or
/// sliding". A *different* rectangle fully on the monitor is a real relocation
/// (taskbar resized, moved to another edge, icon size changed).
pub fn classify(snapshot: &TaskbarSnapshot, observation: &Observation) -> Placement {
    if !observation.exists {
        return Placement::Gone;
    }
    let (Some(rect), Some(monitor)) = (observation.rect, observation.monitor_rect) else {
        return Placement::Gone;
    };
    if monitor != snapshot.monitor_rect || (observation.dpi != 0 && observation.dpi != snapshot.dpi)
    {
        return Placement::Relocated;
    }
    if !observation.visible {
        return Placement::Present(TaskbarVisibility::Hidden);
    }
    if rect == snapshot.taskbar_rect {
        Placement::Present(TaskbarVisibility::Visible)
    } else if monitor.contains(rect) {
        Placement::Relocated
    } else {
        Placement::Present(TaskbarVisibility::Hidden)
    }
}

/// A foreground window that covers an entire monitor hides that monitor's
/// overlay, unless it is the desktop or the shell itself.
pub fn covers_monitor(
    foreground: Option<PhysicalRect>,
    foreground_is_shell: bool,
    monitor: PhysicalRect,
) -> bool {
    !foreground_is_shell && foreground.is_some_and(|rect| rect.contains(monitor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clock_core::*;

    fn rect(l: i32, t: i32, w: i32, h: i32) -> PhysicalRect {
        PhysicalRect::new(l, t, w, h).unwrap()
    }
    fn snapshot() -> TaskbarSnapshot {
        TaskbarSnapshot {
            key: TaskbarKey {
                generation: ShellGeneration(1),
                id: TaskbarId(1),
            },
            taskbar_rect: rect(-1920, 1032, 1920, 48),
            clock_rect: None,
            monitor: MonitorId(1),
            monitor_rect: rect(-1920, 0, 1920, 1080),
            dpi: 144,
            primary: false,
            visibility: TaskbarVisibility::Visible,
            strategy: DiscoveryStrategy::NativeChildren,
        }
    }
    fn observed(s: &TaskbarSnapshot) -> Observation {
        Observation {
            exists: true,
            visible: true,
            rect: Some(s.taskbar_rect),
            dpi: s.dpi,
            monitor_rect: Some(s.monitor_rect),
            fullscreen: false,
        }
    }

    #[test]
    fn unchanged_negative_coordinate_taskbar_is_visible() {
        let s = snapshot();
        assert_eq!(
            classify(&s, &observed(&s)),
            Placement::Present(TaskbarVisibility::Visible)
        );
    }

    #[test]
    fn auto_hide_slide_and_parked_taskbar_are_hidden_not_relocated() {
        let s = snapshot();
        for rect in [rect(-1920, 1078, 1920, 48), rect(-1920, 1056, 1920, 48)] {
            let o = Observation {
                rect: Some(rect),
                ..observed(&s)
            };
            assert_eq!(
                classify(&s, &o),
                Placement::Present(TaskbarVisibility::Hidden),
                "{rect:?}"
            );
        }
        let o = Observation {
            visible: false,
            ..observed(&s)
        };
        assert_eq!(classify(&s, &o), Placement::Present(TaskbarVisibility::Hidden));
    }

    #[test]
    fn a_resized_or_moved_taskbar_fully_on_the_monitor_needs_rediscovery() {
        let s = snapshot();
        for rect in [rect(-1920, 1000, 1920, 80), rect(-1920, 0, 1920, 48)] {
            let o = Observation {
                rect: Some(rect),
                ..observed(&s)
            };
            assert_eq!(classify(&s, &o), Placement::Relocated, "{rect:?}");
        }
    }

    #[test]
    fn dpi_or_monitor_changes_and_missing_windows_invalidate_the_snapshot() {
        let s = snapshot();
        assert_eq!(
            classify(&s, &Observation { dpi: 96, ..observed(&s) }),
            Placement::Relocated
        );
        assert_eq!(
            classify(
                &s,
                &Observation {
                    monitor_rect: Some(rect(0, 0, 1920, 1080)),
                    ..observed(&s)
                }
            ),
            Placement::Relocated
        );
        assert_eq!(classify(&s, &Observation::GONE), Placement::Gone);
        assert_eq!(
            classify(&s, &Observation { rect: None, ..observed(&s) }),
            Placement::Gone
        );
        // Unknown DPI (zero) is not treated as a change; discovery owns validity.
        assert_eq!(
            classify(&s, &Observation { dpi: 0, ..observed(&s) }),
            Placement::Present(TaskbarVisibility::Visible)
        );
    }

    #[test]
    fn fullscreen_requires_a_covering_non_shell_foreground() {
        let monitor = rect(-1920, 0, 1920, 1080);
        assert!(covers_monitor(Some(monitor), false, monitor));
        assert!(covers_monitor(Some(rect(-2000, -10, 2100, 1200)), false, monitor));
        assert!(!covers_monitor(Some(monitor), true, monitor), "desktop/shell");
        assert!(!covers_monitor(None, false, monitor));
        assert!(!covers_monitor(Some(rect(-1900, 0, 1000, 700)), false, monitor));
        // A fullscreen window on the neighbouring monitor does not cover this one.
        assert!(!covers_monitor(Some(rect(0, 0, 1920, 1080)), false, monitor));
    }
}
