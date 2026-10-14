use chrono::{DateTime, FixedOffset};
use clock_core::*;
use std::time::Duration;

fn wall(value: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(value).unwrap()
}
fn prefs(format: &str) -> ValidatedPreferences {
    Preferences {
        format: format.into(),
        monitors: Monitors::All,
        ..Preferences::default()
    }
    .validate()
    .unwrap()
}
fn snapshot(id: u64, generation: u64) -> TaskbarSnapshot {
    TaskbarSnapshot {
        key: TaskbarKey {
            id: TaskbarId(id),
            generation: ShellGeneration(generation),
        },
        monitor: MonitorId(id),
        monitor_rect: PhysicalRect::new(-1920, -1080, 1920, 1080).unwrap(),
        taskbar_rect: PhysicalRect::new(-1920, -40, 1920, 40).unwrap(),
        clock_rect: Some(PhysicalRect::new(-120, -40, 120, 40).unwrap()),
        dpi: 144,
        primary: id == 1,
        visibility: TaskbarVisibility::Visible,
        strategy: DiscoveryStrategy::NativeChildren,
    }
}
fn discovery(snapshots: Vec<TaskbarSnapshot>, generation: u64) -> Event {
    Event::DiscoveryResult {
        generation: ShellGeneration(generation),
        snapshots,
    }
}
fn ready() -> State {
    let (state, _) = reduce(
        State::new(prefs("%H:%M")),
        Event::Tick(wall("2026-10-14T12:00:00+00:00")),
    );
    reduce(state, discovery(vec![snapshot(1, 0), snapshot(2, 0)], 0)).0
}
fn paints(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Paint { .. }))
        .count()
}

#[test]
fn boundaries_and_clock_jumps_use_fresh_wall_snapshots() {
    for (time, second, minute) in [
        ("2026-10-14T12:00:00+00:00", 1_000, 60_000),
        ("2026-10-14T12:00:59.750+00:00", 250, 250),
        ("1960-01-01T00:00:12.125-05:00", 875, 47_875),
    ] {
        assert_eq!(
            next_delay(wall(time), Cadence::Second),
            Duration::from_millis(second)
        );
        assert_eq!(
            next_delay(wall(time), Cadence::Minute),
            Duration::from_millis(minute)
        );
    }
    let mut state = State::new(prefs("%Y-%m-%d %H:%M:%S"));
    (state, _) = reduce(state, discovery(vec![snapshot(1, 0)], 0));
    // Midnight, backwards jump, forward jump, and explicit DST offset snapshots.
    for time in [
        "2026-03-08T23:59:59.750-04:00",
        "2026-03-09T00:00:00-04:00",
        "2026-03-08T01:59:59-05:00",
        "2026-03-08T03:00:00-04:00",
        "2026-11-01T01:59:59-04:00",
        "2026-11-01T01:00:00-05:00",
    ] {
        let snapshot = wall(time);
        let effects;
        (state, effects) = reduce(state, Event::TimeChanged(snapshot));
        assert!(
            effects.contains(&Effect::Paint {
                key: snapshot_key(),
                text: prefs("%Y-%m-%d %H:%M:%S")
                    .format()
                    .render(snapshot)
                    .unwrap()
            })
        );
        assert!(effects.contains(&Effect::ArmTimer(next_delay(snapshot, Cadence::Second))));
    }
    assert_eq!(prefs("%Y-%m-%d").format().cadence(), Cadence::Minute);
}
fn snapshot_key() -> TaskbarKey {
    snapshot(1, 0).key
}

#[test]
fn independent_visibility_and_idempotent_events() {
    let mut state = ready();
    for event in [
        Event::VisibilityChanged {
            key: snapshot_key(),
            visibility: TaskbarVisibility::Hidden,
        },
        Event::FullscreenChanged {
            key: snapshot(2, 0).key,
            fullscreen: true,
        },
        Event::SessionLocked,
    ] {
        let effects;
        (state, effects) = reduce(state, event.clone());
        assert_eq!(paints(&effects), 0);
        let repeated;
        (state, repeated) = reduce(state, event);
        assert!(repeated.is_empty());
    }
    assert_eq!(
        state.overlay_state(snapshot_key()),
        OverlayState::Hidden(HiddenReason::SessionLocked)
    );
    let (state, effects) = reduce(state, Event::SessionUnlocked);
    assert_eq!(paints(&effects), 0);
    assert!(effects.contains(&Effect::Discover(ShellGeneration(0))));
    let (state, effects) = reduce(state, discovery(vec![snapshot(1, 0), snapshot(2, 0)], 0));
    assert_eq!(paints(&effects), 0); // no pre-lock wall snapshot
    let (state, effects) = reduce(state, Event::Tick(wall("2026-10-14T12:01:00+00:00")));
    assert_eq!(paints(&effects), 1); // fullscreen persists for second taskbar
    assert_eq!(
        state.overlay_state(snapshot(2, 0).key),
        OverlayState::Hidden(HiddenReason::Fullscreen)
    );
}

#[test]
fn reconcile_geometry_theme_and_preferences_without_duplicate_windows() {
    let mut state = ready();
    let (next, effects) = reduce(state, discovery(vec![snapshot(2, 0), snapshot(1, 0)], 0));
    state = next;
    assert!(effects.is_empty());
    let mut changed = snapshot(1, 0);
    changed.clock_rect = Some(PhysicalRect::new(-150, -40, 150, 40).unwrap());
    let effects;
    (state, effects) = reduce(state, discovery(vec![changed.clone()], 0));
    assert!(effects.contains(&Effect::DestroyOverlay(snapshot(2, 0).key)));
    assert!(effects.contains(&Effect::SetBounds {
        key: changed.key,
        bounds: changed.clock_rect.unwrap()
    }));
    assert_eq!(paints(&effects), 1);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::CreateOverlay { .. }))
    );
    (state, _) = reduce(state, Event::ThemeChanged { revision: 1 });
    let (next, effects) = reduce(state, Event::ThemeChanged { revision: 1 });
    state = next;
    assert!(effects.is_empty());
    let (next, effects) = reduce(state, Event::Tick(wall("2026-10-14T12:00:30+00:00")));
    state = next;
    assert_eq!(paints(&effects), 0);
    assert_eq!(effects, vec![Effect::ArmTimer(Duration::from_secs(30))]);
    let (state, effects) = reduce(state, Event::PreferencesApplied(prefs("%H:%M:%S")));
    assert_eq!(paints(&effects), 1);
    assert!(effects.contains(&Effect::ArmTimer(Duration::from_secs(1))));
    let (_, effects) = reduce(state, Event::PreferencesApplied(prefs("%H:%M:%S")));
    assert!(effects.is_empty());
}

#[test]
fn unsafe_or_unknown_discovery_never_creates_overlays() {
    for variant in 0..5 {
        let mut bar = snapshot(1, 0);
        match variant {
            0 => bar.clock_rect = None,
            1 => bar.visibility = TaskbarVisibility::Unknown,
            2 => bar.dpi = 0,
            3 => bar.clock_rect = Some(PhysicalRect::new(0, 0, 10, 10).unwrap()),
            _ => bar.visibility = TaskbarVisibility::Hidden,
        }
        let (state, _) = reduce(
            State::new(prefs("%H:%M")),
            Event::Tick(wall("2026-10-14T12:00:00+00:00")),
        );
        let (_, effects) = reduce(state, discovery(vec![bar], 0));
        assert!(effects.is_empty());
    }
    let (state, _) = reduce(ready(), Event::SessionLocked);
    let (_, effects) = reduce(state, discovery(vec![snapshot(3, 0)], 0));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::CreateOverlay { .. } | Effect::Paint { .. }))
    );
}

#[test]
fn explorer_loss_stale_batches_and_quit() {
    let (state, effects) = reduce(ready(), Event::ExplorerLost);
    assert_eq!(
        effects,
        vec![
            Effect::DestroyOverlay(snapshot(1, 0).key),
            Effect::DestroyOverlay(snapshot(2, 0).key),
            Effect::Discover(ShellGeneration(1))
        ]
    );
    let (state, effects) = reduce(state, Event::ExplorerLost);
    assert!(effects.is_empty());
    let (state, effects) = reduce(state, discovery(vec![snapshot(1, 0)], 0));
    assert!(effects.is_empty());
    let (state, effects) = reduce(state, discovery(vec![snapshot(1, 0)], 1));
    assert!(effects.is_empty()); // mixed generation batch
    let (state, effects) = reduce(state, discovery(vec![snapshot(1, 1), snapshot(1, 1)], 1));
    assert!(effects.is_empty()); // duplicate batch rejected
    let (state, effects) = reduce(state, discovery(vec![snapshot(1, 1)], 1));
    assert_eq!(paints(&effects), 1);
    assert_eq!(
        state.overlay_state(snapshot_key()),
        OverlayState::Unavailable
    );
    let (state, effects) = reduce(state, Event::Quit);
    assert_eq!(effects, vec![Effect::DestroyOverlay(snapshot(1, 1).key)]);
    let (_, effects) = reduce(state, Event::TopologyChanged);
    assert!(effects.is_empty());
}

proptest::proptest! {
    #[test]
    fn monotonic_wait_is_positive_and_bounded(second in 0u32..60, nanos in 0u32..1_000_000_000) {
        use chrono::Timelike;
        let time = wall("2026-10-14T12:00:00+00:00").with_second(second).unwrap().with_nanosecond(nanos).unwrap();
        for (cadence, bound) in [(Cadence::Second, 1), (Cadence::Minute, 60)] {
            let delay = next_delay(time, cadence);
            proptest::prop_assert!(delay > Duration::ZERO && delay <= Duration::from_secs(bound));
            let after = time + chrono::TimeDelta::from_std(delay).unwrap();
            proptest::prop_assert_eq!(after.nanosecond(), 0);
            if cadence == Cadence::Minute { proptest::prop_assert_eq!(after.second(), 0); }
        }
    }
}

#[test]
fn topology_preserves_windows_and_hidden_changes_defer_paint() {
    let (state, effects) = reduce(ready(), Event::TopologyChanged);
    assert_eq!(effects, vec![Effect::Discover(ShellGeneration(0))]);
    let (state, effects) = reduce(state, Event::TopologyChanged);
    assert!(effects.is_empty());
    let (state, _) = reduce(state, discovery(vec![snapshot(1, 0)], 0));
    let (state, _) = reduce(
        state,
        Event::FullscreenChanged {
            key: snapshot_key(),
            fullscreen: true,
        },
    );
    let (state, effects) = reduce(state, Event::ThemeChanged { revision: 2 });
    assert!(effects.is_empty());
    let (_, effects) = reduce(
        state,
        Event::FullscreenChanged {
            key: snapshot_key(),
            fullscreen: false,
        },
    );
    assert_eq!(paints(&effects), 1);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::CreateOverlay { .. }))
    );
}

proptest::proptest! {
    #[test]
    fn event_sequences_preserve_visibility_and_window_lifetime(events in proptest::collection::vec(0u8..10, 1..100)) {
        let mut state = State::new(prefs("%H:%M"));
        let mut windows = std::collections::HashSet::new();
        for value in events {
            let key = snapshot(1, state.generation().0).key;
            let event = match value {
                0 => discovery(vec![snapshot(1, key.generation.0)], key.generation.0),
                1 => Event::VisibilityChanged {key, visibility: TaskbarVisibility::Hidden},
                2 => Event::FullscreenChanged {key, fullscreen: true},
                3 => Event::SessionLocked,
                4 => Event::SessionUnlocked,
                5 => Event::ExplorerLost,
                6 => Event::Tick(wall("2026-10-14T12:00:00+00:00")),
                7 => Event::SetPaused(true),
                8 => Event::SetPaused(false),
                _ => Event::TopologyChanged,
            };
            let effects;
            (state, effects) = reduce(state, event);
            for effect in effects {
                match effect {
                    Effect::CreateOverlay {key, ..} => { proptest::prop_assert!(windows.insert(key)); }
                    Effect::DestroyOverlay(key) => { proptest::prop_assert!(windows.remove(&key)); }
                    Effect::Paint {key, ..} => {
                        proptest::prop_assert!(windows.contains(&key));
                        proptest::prop_assert_eq!(state.overlay_state(key), OverlayState::Visible);
                    }
                    Effect::Hide(key) | Effect::SetBounds {key, ..} => { proptest::prop_assert!(windows.contains(&key)); }
                    _ => {}
                }
            }
        }
    }
}

#[test]
fn date_only_rechecks_minutes_and_paints_at_local_midnight() {
    let (state, _) = reduce(
        State::new(prefs("%Y-%m-%d")),
        discovery(vec![snapshot(1, 0)], 0),
    );
    let (state, effects) = reduce(state, Event::Tick(wall("2026-10-14T23:59:59.900+05:30")));
    assert_eq!(paints(&effects), 1);
    assert!(effects.contains(&Effect::ArmTimer(Duration::from_millis(100))));
    let (state, effects) = reduce(state, Event::Tick(wall("2026-10-15T00:00:00+05:30")));
    assert!(effects.contains(&Effect::Paint {
        key: snapshot_key(),
        text: ClockText {
            lines: vec!["2026-10-15".into()]
        }
    }));
    let (_, effects) = reduce(state, Event::Tick(wall("2026-10-15T00:01:00+05:30")));
    assert_eq!(effects, vec![Effect::ArmTimer(Duration::from_secs(60))]);
}

#[test]
fn hiding_one_monitor_does_not_hide_the_other() {
    let (state, effects) = reduce(
        ready(),
        Event::VisibilityChanged {
            key: snapshot_key(),
            visibility: TaskbarVisibility::Hidden,
        },
    );
    assert_eq!(effects, vec![Effect::Hide(snapshot_key())]);
    assert_eq!(
        state.overlay_state(snapshot(2, 0).key),
        OverlayState::Visible
    );
    let (_, effects) = reduce(
        state,
        Event::VisibilityChanged {
            key: snapshot(9, 0).key,
            visibility: TaskbarVisibility::Visible,
        },
    );
    assert!(effects.is_empty());
}

#[test]
fn out_of_range_local_snapshot_removes_overlay_without_panicking() {
    let offset = FixedOffset::east_opt(3600).unwrap();
    let extreme = DateTime::from_naive_utc_and_offset(chrono::NaiveDateTime::MAX, offset);
    let (state, effects) = reduce(ready(), Event::TimeChanged(extreme));
    assert_eq!(
        state.overlay_state(snapshot_key()),
        OverlayState::Unavailable
    );
    assert!(effects.contains(&Effect::DestroyOverlay(snapshot_key())));
    assert_eq!(paints(&effects), 0);
    assert!(next_delay(extreme, Cadence::Minute) > Duration::ZERO);
}

#[test]
fn pause_hides_every_overlay_and_resume_repaints_without_new_windows() {
    let state = ready();
    let (state, effects) = reduce(state, Event::SetPaused(true));
    assert!(state.is_paused());
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Hide(_)))
            .count(),
        2
    );
    assert!(!effects.iter().any(|e| matches!(
        e,
        Effect::CreateOverlay { .. } | Effect::DestroyOverlay(_) | Effect::Paint { .. }
    )));
    for id in [1, 2] {
        assert_eq!(
            state.overlay_state(snapshot(id, 0).key),
            OverlayState::Hidden(HiddenReason::Paused)
        );
    }
    // Pausing twice and ticking while paused neither re-hides nor paints.
    let (state, repeated) = reduce(state, Event::SetPaused(true));
    assert!(repeated.is_empty());
    let (state, ticked) = reduce(state, Event::Tick(wall("2026-10-14T12:01:00+00:00")));
    assert_eq!(paints(&ticked), 0);
    assert!(!ticked.iter().any(|e| matches!(e, Effect::Hide(_))));
    // Discovery results keep the pause: a rediscovered taskbar stays hidden.
    let (state, effects) = reduce(state, discovery(vec![snapshot(1, 0), snapshot(2, 0)], 0));
    assert_eq!(paints(&effects), 0);
    let (state, resumed) = reduce(state, Event::SetPaused(false));
    assert!(!state.is_paused());
    assert_eq!(
        paints(&resumed),
        2,
        "resume shows current text once per clock"
    );
    assert!(
        !resumed
            .iter()
            .any(|e| matches!(e, Effect::CreateOverlay { .. }))
    );
}

#[test]
fn explorer_loss_while_paused_stays_paused_and_quit_is_unaffected() {
    let (state, _) = reduce(ready(), Event::SetPaused(true));
    let (state, _) = reduce(state, Event::ExplorerLost);
    assert!(state.is_paused(), "pause is a user choice, not shell state");
    let (state, _) = reduce(state, Event::Quit);
    assert!(state.is_quitting());
}
