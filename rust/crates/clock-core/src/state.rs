//! Pure UI-effect reducer. Discovery batches are authoritative for one generation.
use crate::{
    ClockText, Effect, Event, HiddenReason, Monitors, OverlayState, PhysicalRect, ShellGeneration,
    TaskbarKey, TaskbarSnapshot, TaskbarVisibility, ValidatedPreferences, next_delay,
};
use chrono::{DateTime, FixedOffset};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct Taskbar {
    snapshot: TaskbarSnapshot,
    fullscreen: bool,
    status: OverlayState,
    bounds: Option<PhysicalRect>,
    painted: Option<ClockText>,
    dirty: bool,
}

/// Owns values only. Inspect visibility with `overlay_state`; native resources
/// belong to the runtime. A fresh State starts at generation zero with no time.
#[derive(Clone, Debug)]
pub struct State {
    generation: ShellGeneration,
    taskbars: BTreeMap<u64, Taskbar>,
    preferences: ValidatedPreferences,
    wall: Option<DateTime<FixedOffset>>,
    theme_revision: u64,
    locked: bool,
    paused: bool,
    discovering: bool,
    quitting: bool,
    shell_lost: bool,
}
impl State {
    pub fn new(preferences: ValidatedPreferences) -> Self {
        Self {
            generation: ShellGeneration(0),
            taskbars: BTreeMap::new(),
            preferences,
            wall: None,
            theme_revision: 0,
            locked: false,
            paused: false,
            discovering: false,
            quitting: false,
            shell_lost: false,
        }
    }
    /// Committed preferences used to apply appearance effects on the UI thread.
    pub fn preferences(&self) -> &ValidatedPreferences {
        &self.preferences
    }
    /// True while the user has paused clock replacement; every overlay is hidden.
    pub fn is_paused(&self) -> bool {
        self.paused
    }
    pub fn is_quitting(&self) -> bool {
        self.quitting
    }
    pub fn generation(&self) -> ShellGeneration {
        self.generation
    }
    pub fn overlay_state(&self, key: TaskbarKey) -> OverlayState {
        if key.generation != self.generation {
            return OverlayState::Unavailable;
        }
        self.taskbars
            .get(&key.id.0)
            .map_or(OverlayState::Unavailable, |bar| bar.status)
    }
    fn discover(&mut self, effects: &mut Vec<Effect>) {
        if !self.discovering {
            self.discovering = true;
            effects.push(Effect::Discover(self.generation));
        }
    }
    fn reconcile(&mut self, effects: &mut Vec<Effect>) {
        let text = self
            .wall
            .and_then(|wall| self.preferences.format().render(wall).ok());
        for bar in self.taskbars.values_mut() {
            let snapshot = &bar.snapshot;
            let key = snapshot.key;
            let geometry = snapshot.clock_rect.filter(|rect| {
                snapshot.dpi > 0
                    && snapshot.taskbar_rect.contains(*rect)
                    && snapshot.monitor_rect.contains(snapshot.taskbar_rect)
            });
            let status = if geometry.is_none()
                || snapshot.visibility == TaskbarVisibility::Unknown
                || text.is_none()
            {
                OverlayState::Unavailable
            } else if self.locked {
                OverlayState::Hidden(HiddenReason::SessionLocked)
            } else if self.paused
                || (self.preferences.monitors() == Monitors::Primary && !snapshot.primary)
            {
                OverlayState::Hidden(HiddenReason::Paused)
            } else if snapshot.visibility == TaskbarVisibility::Hidden {
                OverlayState::Hidden(HiddenReason::TaskbarHidden)
            } else if bar.fullscreen {
                OverlayState::Hidden(HiddenReason::Fullscreen)
            } else {
                OverlayState::Visible
            };
            if status == OverlayState::Unavailable {
                if bar.bounds.take().is_some() {
                    effects.push(Effect::DestroyOverlay(key));
                }
                bar.painted = None;
            } else if status == OverlayState::Visible {
                let bounds = geometry.expect("visible requires validated bounds");
                if bar.bounds.is_none() {
                    effects.push(Effect::CreateOverlay { key, bounds });
                } else if bar.bounds != Some(bounds) {
                    effects.push(Effect::SetBounds { key, bounds });
                    bar.dirty = true;
                }
                bar.bounds = Some(bounds);
                let text = text.as_ref().expect("visible requires rendered text");
                if bar.status != OverlayState::Visible
                    || bar.dirty
                    || bar.painted.as_ref() != Some(text)
                {
                    effects.push(Effect::Paint {
                        key,
                        text: text.clone(),
                    });
                    bar.painted = Some(text.clone());
                    bar.dirty = false;
                }
            } else if bar.status == OverlayState::Visible && bar.bounds.is_some() {
                effects.push(Effect::Hide(key));
            }
            bar.status = status;
        }
    }
}

/// Effects are ordered by transient ID. Create is initially hidden; Paint also
/// shows an eligible overlay. The runtime uses current committed preferences and
/// resolved appearance when painting. ArmTimer replaces the outstanding one-shot
/// monotonic wait. Quit cancels runtime timers; late events become no-ops.
pub fn reduce(mut state: State, event: Event) -> (State, Vec<Effect>) {
    let mut effects = Vec::new();
    if state.quitting {
        return (state, effects);
    }
    let mut arm = false;
    match event {
        Event::TopologyChanged => state.discover(&mut effects),
        Event::DiscoveryResult {
            generation,
            snapshots,
        } => {
            if generation != state.generation
                || snapshots.iter().any(|s| s.key.generation != generation)
            {
                return (state, effects);
            }
            // Reject duplicate keys atomically, avoiding order-dependent geometry.
            let mut batch = BTreeMap::new();
            for snapshot in snapshots {
                if batch.insert(snapshot.key.id.0, snapshot).is_some() {
                    return (state, effects);
                }
            }
            state.discovering = false;
            state.shell_lost = false;
            state.taskbars.retain(|id, bar| {
                if batch.contains_key(id) {
                    return true;
                }
                if bar.bounds.is_some() {
                    effects.push(Effect::DestroyOverlay(bar.snapshot.key));
                }
                false
            });
            for (id, snapshot) in batch {
                if let Some(bar) = state.taskbars.get_mut(&id) {
                    bar.snapshot = snapshot;
                } else {
                    state.taskbars.insert(
                        id,
                        Taskbar {
                            snapshot,
                            fullscreen: false,
                            status: OverlayState::Unavailable,
                            bounds: None,
                            painted: None,
                            dirty: true,
                        },
                    );
                }
            }
        }
        Event::VisibilityChanged { key, visibility } => {
            if key.generation != state.generation {
                return (state, effects);
            }
            if let Some(bar) = state.taskbars.get_mut(&key.id.0) {
                bar.snapshot.visibility = visibility;
            }
        }
        Event::FullscreenChanged { key, fullscreen } => {
            if key.generation != state.generation {
                return (state, effects);
            }
            if let Some(bar) = state.taskbars.get_mut(&key.id.0) {
                bar.fullscreen = fullscreen;
            }
        }
        Event::Tick(wall) | Event::TimeChanged(wall) => {
            // Even an identical snapshot must rearm: the one-shot may have fired.
            state.wall = Some(wall);
            arm = true;
        }
        Event::ThemeChanged { revision } => {
            if revision != state.theme_revision {
                state.theme_revision = revision;
                for bar in state.taskbars.values_mut() {
                    bar.dirty = true;
                }
            }
        }
        Event::PreferencesApplied(preferences) => {
            if state.preferences != preferences {
                let appearance_changed = state.preferences.theme() != preferences.theme()
                    || state.preferences.font_size_pt() != preferences.font_size_pt();
                state.preferences = preferences;
                for bar in state.taskbars.values_mut() {
                    bar.dirty |= appearance_changed;
                }
                arm = true;
            }
        }
        Event::SessionLocked => state.locked = true,
        Event::SetPaused(paused) => state.paused = paused,
        Event::SessionUnlocked => {
            if state.locked {
                state.locked = false;
                // Do not show pre-lock geometry/time while awaiting fresh inputs.
                for bar in state.taskbars.values_mut() {
                    bar.snapshot.visibility = TaskbarVisibility::Unknown;
                }
                state.wall = None;
                state.discover(&mut effects);
            }
        }
        Event::ExplorerLost | Event::Quit => {
            if event == Event::ExplorerLost && state.shell_lost {
                return (state, effects);
            }
            state.shell_lost = true;
            for bar in state.taskbars.values() {
                if bar.bounds.is_some() {
                    effects.push(Effect::DestroyOverlay(bar.snapshot.key));
                }
            }
            state.taskbars.clear();
            state.discovering = false;
            if event == Event::Quit {
                state.quitting = true;
            } else if let Some(generation) = state.generation.0.checked_add(1) {
                state.generation = ShellGeneration(generation);
                state.discover(&mut effects);
            } else {
                // Exhaustion cannot safely reuse identities.
                state.quitting = true;
            }
        }
    }
    state.reconcile(&mut effects);
    if arm && let Some(wall) = state.wall {
        effects.push(Effect::ArmTimer(next_delay(
            wall,
            state.preferences.format().cadence(),
        )));
    }
    (state, effects)
}
