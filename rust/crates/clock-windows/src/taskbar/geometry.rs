use clock_core::{PhysicalRect, ShellGeneration, TaskbarSnapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rejection {
    StaleGeneration,
    InvalidDpi,
    OutsideMonitor,
    OutsideTaskbar,
    AdjacentControlOverlap,
    UnverifiedSelector,
    IncompleteExclusions,
}

/// Validate a candidate against a complete inventory of neighboring hit targets.
/// This is a geometric check, not proof of the selector's semantic identity.
pub fn validate_candidate(
    snapshot: &TaskbarSnapshot,
    current: ShellGeneration,
    candidate: PhysicalRect,
    adjacent: &[PhysicalRect],
    exclusions_complete: bool,
) -> Result<(), Rejection> {
    if snapshot.key.generation != current {
        return Err(Rejection::StaleGeneration);
    }
    if snapshot.dpi == 0 {
        return Err(Rejection::InvalidDpi);
    }
    if !snapshot.monitor_rect.contains(snapshot.taskbar_rect)
        || !snapshot.monitor_rect.contains(candidate)
    {
        return Err(Rejection::OutsideMonitor);
    }
    if !snapshot.taskbar_rect.contains(candidate) {
        return Err(Rejection::OutsideTaskbar);
    }
    if !exclusions_complete {
        return Err(Rejection::IncompleteExclusions);
    }
    if adjacent.iter().any(|other| {
        candidate.left() < other.right()
            && other.left() < candidate.right()
            && candidate.top() < other.bottom()
            && other.top() < candidate.bottom()
    }) {
        return Err(Rejection::AdjacentControlOverlap);
    }
    Ok(())
}

/// Fail closed until the manual build/selector/exclusion evidence gate is met.
/// No public boolean allows a caller to declare an experimental selector proven.
pub fn verified_clock_rect(
    snapshot: &TaskbarSnapshot,
    current: ShellGeneration,
    candidate: PhysicalRect,
    adjacent: &[PhysicalRect],
    exclusions_complete: bool,
) -> Result<PhysicalRect, Rejection> {
    validate_candidate(snapshot, current, candidate, adjacent, exclusions_complete)?;
    Err(Rejection::UnverifiedSelector)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clock_core::*;

    fn snapshot() -> TaskbarSnapshot {
        TaskbarSnapshot {
            key: TaskbarKey {
                generation: ShellGeneration(7),
                id: TaskbarId(1),
            },
            taskbar_rect: PhysicalRect::new(-1920, -48, 1920, 48).unwrap(),
            clock_rect: None,
            monitor: MonitorId(1),
            monitor_rect: PhysicalRect::new(-1920, -1080, 1920, 1080).unwrap(),
            dpi: 144,
            primary: false,
            visibility: TaskbarVisibility::Visible,
            strategy: DiscoveryStrategy::NativeChildren,
        }
    }

    #[test]
    fn negative_coordinates_and_touching_edges_are_valid() {
        let s = snapshot();
        let clock = PhysicalRect::new(-120, -48, 80, 48).unwrap();
        let bell = PhysicalRect::new(-40, -48, 24, 48).unwrap();
        assert_eq!(
            validate_candidate(&s, ShellGeneration(7), clock, &[bell], true),
            Ok(())
        );
        assert_eq!(
            verified_clock_rect(&s, ShellGeneration(7), clock, &[bell], true),
            Err(Rejection::UnverifiedSelector)
        );
    }

    #[test]
    fn rejects_bell_show_desktop_and_partial_overlap() {
        let s = snapshot();
        let clock = PhysicalRect::new(-120, -48, 120, 48).unwrap();
        for neighbor in [
            PhysicalRect::new(-40, -48, 24, 48).unwrap(),
            PhysicalRect::new(-4, -48, 4, 48).unwrap(),
        ] {
            assert_eq!(
                validate_candidate(&s, ShellGeneration(7), clock, &[neighbor], true),
                Err(Rejection::AdjacentControlOverlap)
            );
        }
    }

    #[test]
    fn rejects_stale_generation_unknown_dpi_and_incomplete_inventory() {
        let mut s = snapshot();
        let c = PhysicalRect::new(-120, -48, 80, 48).unwrap();
        assert_eq!(
            validate_candidate(&s, ShellGeneration(8), c, &[], true),
            Err(Rejection::StaleGeneration)
        );
        assert_eq!(
            validate_candidate(&s, ShellGeneration(7), c, &[], false),
            Err(Rejection::IncompleteExclusions)
        );
        s.dpi = 0;
        assert_eq!(
            validate_candidate(&s, ShellGeneration(7), c, &[], true),
            Err(Rejection::InvalidDpi)
        );
    }

    #[test]
    fn rejects_bounds_outside_taskbar_or_monitor() {
        let mut s = snapshot();
        assert_eq!(
            validate_candidate(
                &s,
                ShellGeneration(7),
                PhysicalRect::new(-120, -60, 80, 48).unwrap(),
                &[],
                true
            ),
            Err(Rejection::OutsideTaskbar)
        );
        assert_eq!(
            validate_candidate(
                &s,
                ShellGeneration(7),
                PhysicalRect::new(-40, -48, 80, 48).unwrap(),
                &[],
                true
            ),
            Err(Rejection::OutsideMonitor)
        );
        s.taskbar_rect = PhysicalRect::new(-1921, -48, 1920, 48).unwrap();
        assert_eq!(
            validate_candidate(
                &s,
                ShellGeneration(7),
                PhysicalRect::new(-120, -48, 80, 48).unwrap(),
                &[],
                true
            ),
            Err(Rejection::OutsideMonitor)
        );
        assert!(PhysicalRect::new(0, 0, -1, 10).is_err());
        assert!(PhysicalRect::new(i32::MAX, 0, 1, 10).is_err());
    }
}
