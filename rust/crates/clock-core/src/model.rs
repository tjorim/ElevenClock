//! Contracts for R02 discovery, R04 formatting, R05 preferences and R06 state.
use crate::GeometryError;
use chrono::{DateTime, FixedOffset};
use std::time::Duration;

/// Physical pixels, including negative coordinates on secondary monitors.
/// Private edges ensure construction always validates positive dimensions.
/// ```
/// use clock_core::PhysicalRect;
/// let rect = PhysicalRect::new(-1920, 0, 1920, 1080)?;
/// assert_eq!(rect.right(), 0);
/// # Ok::<(), clock_core::GeometryError>(())
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}
impl PhysicalRect {
    pub fn new(left: i32, top: i32, width: i32, height: i32) -> Result<Self, GeometryError> {
        if width <= 0 || height <= 0 {
            return Err(GeometryError::NonPositiveSize);
        }
        let right = left.checked_add(width).ok_or(GeometryError::EdgeOverflow)?;
        let bottom = top.checked_add(height).ok_or(GeometryError::EdgeOverflow)?;
        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }
    pub const fn left(self) -> i32 {
        self.left
    }
    pub const fn top(self) -> i32 {
        self.top
    }
    pub const fn right(self) -> i32 {
        self.right
    }
    pub const fn bottom(self) -> i32 {
        self.bottom
    }
    pub const fn width(self) -> i32 {
        self.right - self.left
    }
    pub const fn height(self) -> i32 {
        self.bottom - self.top
    }
    pub fn contains(self, other: Self) -> bool {
        other.left >= self.left
            && other.top >= self.top
            && other.right <= self.right
            && other.bottom <= self.bottom
    }
}

/// Transient value assigned by discovery; never a HWND or persisted identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TaskbarId(pub u64);
/// Increment on Explorer loss/replacement; stale results must be discarded.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ShellGeneration(pub u64);
/// Every taskbar reference is scoped to an Explorer generation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TaskbarKey {
    pub generation: ShellGeneration,
    pub id: TaskbarId,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MonitorId(pub u64);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryStrategy {
    NativeChildren,
    UiAutomation,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskbarVisibility {
    Visible,
    Hidden,
    Unknown,
}

/// A clock rect is Some only after clock-only geometry has been proven.
/// R02 validates monitor containment, DPI and exclusion of adjacent controls.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskbarSnapshot {
    pub key: TaskbarKey,
    pub taskbar_rect: PhysicalRect,
    pub clock_rect: Option<PhysicalRect>,
    pub monitor: MonitorId,
    pub monitor_rect: PhysicalRect,
    pub dpi: u32,
    pub primary: bool,
    pub visibility: TaskbarVisibility,
    pub strategy: DiscoveryStrategy,
}

/// Owned rendered lines. R04 enforces <=2 nonempty lines and <=512 scalars.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClockText {
    pub lines: Vec<String>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cadence {
    Second,
    Minute,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MonitorSelection {
    Primary,
    All,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Theme {
    System,
    Light,
    Dark,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HiddenReason {
    TaskbarHidden,
    Fullscreen,
    SessionLocked,
    Paused,
    InvalidLayout,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayState {
    Unavailable,
    Hidden(HiddenReason),
    Visible,
}

/// Value-only inputs; each time event carries one OS-local snapshot for all clocks.
/// PreferencesApplied carries R05's committed validated preferences. Theme revisions
/// identify resolved platform appearance changes, including high contrast.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    TopologyChanged,
    DiscoveryResult {
        generation: ShellGeneration,
        snapshots: Vec<TaskbarSnapshot>,
    },
    VisibilityChanged {
        key: TaskbarKey,
        visibility: TaskbarVisibility,
    },
    Tick(DateTime<FixedOffset>),
    TimeChanged(DateTime<FixedOffset>),
    FullscreenChanged {
        key: TaskbarKey,
        fullscreen: bool,
    },
    ThemeChanged {
        revision: u64,
    },
    PreferencesApplied(crate::ValidatedPreferences),
    ExplorerLost,
    SessionLocked,
    SessionUnlocked,
    /// The user paused or resumed clock replacement from the clock/tray menu.
    /// Session-only: a fresh `State` is never paused.
    SetPaused(bool),
    Quit,
}
/// UI-thread effects. No effect carries native resources across a worker channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    Discover(ShellGeneration),
    CreateOverlay {
        key: TaskbarKey,
        bounds: PhysicalRect,
    },
    DestroyOverlay(TaskbarKey),
    SetBounds {
        key: TaskbarKey,
        bounds: PhysicalRect,
    },
    Paint {
        key: TaskbarKey,
        text: ClockText,
    },
    Hide(TaskbarKey),
    ArmTimer(Duration),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_geometry_and_accepts_negative_monitor_coordinates() {
        assert_eq!(
            PhysicalRect::new(0, 0, 0, 1),
            Err(GeometryError::NonPositiveSize)
        );
        assert_eq!(
            PhysicalRect::new(0, 0, 1, -1),
            Err(GeometryError::NonPositiveSize)
        );
        assert_eq!(
            PhysicalRect::new(i32::MAX, 0, 1, 1),
            Err(GeometryError::EdgeOverflow)
        );
        assert_eq!(
            PhysicalRect::new(0, i32::MAX, 1, 1),
            Err(GeometryError::EdgeOverflow)
        );
        let monitor = PhysicalRect::new(-1920, -1080, 1920, 1080).unwrap();
        assert!(monitor.contains(PhysicalRect::new(-200, -40, 100, 30).unwrap()));
        assert!(!monitor.contains(PhysicalRect::new(-50, -40, 100, 30).unwrap()));
        assert_eq!((monitor.width(), monitor.height()), (1920, 1080));
    }
}
