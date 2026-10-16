//! Deterministic coordinator tests. No test sleeps, reads live time or touches
//! the OS: every platform answer is scripted, so each rule is checked on any host.
use super::*;
use crate::{
    events::{ShellEvent, Signal},
    interaction::{ACTIVATION_COOLDOWN, CalendarOutcome, MenuCommand, ScreenPoint},
    render::PixelSize,
};
use chrono::DateTime;
use clock_core::{DiscoveryStrategy, MonitorId, Monitors, Preferences, TaskbarId, Theme};
use std::collections::{BTreeMap, HashSet};

fn wall(value: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(value).unwrap()
}
fn rect(l: i32, t: i32, w: i32, h: i32) -> PhysicalRect {
    PhysicalRect::new(l, t, w, h).unwrap()
}
fn prefs(monitors: Monitors) -> ValidatedPreferences {
    Preferences {
        monitors,
        ..Preferences::default()
    }
    .validate()
    .unwrap()
}
fn key(generation: u64, id: u64) -> TaskbarKey {
    TaskbarKey {
        generation: ShellGeneration(generation),
        id: TaskbarId(id),
    }
}
/// id 1: primary at the origin; id 2: secondary left of it (negative coordinates).
fn bar(generation: u64, id: u64, clock: bool) -> TaskbarSnapshot {
    let (left, primary) = if id == 1 { (0, true) } else { (-1920, false) };
    TaskbarSnapshot {
        key: key(generation, id),
        taskbar_rect: rect(left, 1032, 1920, 48),
        clock_rect: clock.then(|| rect(left + 1800, 1032, 100, 48)),
        monitor: MonitorId(id),
        monitor_rect: rect(left, 0, 1920, 1080),
        dpi: if id == 1 { 96 } else { 144 },
        primary,
        visibility: TaskbarVisibility::Visible,
        strategy: DiscoveryStrategy::NativeChildren,
    }
}
fn shell(pid: u32, started: u64) -> Option<ShellIdentity> {
    Some(ShellIdentity { pid, started })
}

#[derive(Clone, Debug, PartialEq)]
enum Call {
    RequestDiscovery(u64),
    Create(TaskbarKey),
    SetBounds(TaskbarKey),
    Destroy(TaskbarKey),
    Paint(TaskbarKey, Vec<String>),
    Hide(TaskbarKey),
    Timer(Duration),
    Poll(Duration),
    ShellChanged(Option<ShellIdentity>),
    OpenCalendar(TaskbarKey),
    ShowMenu(Option<ScreenPoint>, bool),
    OpenSettings,
    RestoreTray,
    PausedChanged(bool),
    Stop,
}

struct Fake {
    calls: Vec<Call>,
    mono: Duration,
    now: DateTime<FixedOffset>,
    appearance: SystemAppearance,
    font_changed: bool,
    shell: Option<ShellIdentity>,
    observations: BTreeMap<u64, Observation>,
    overlays: HashSet<TaskbarKey>,
    paint_result: BTreeMap<u64, PaintResult>,
    create_error: bool,
    calendar: CalendarOutcome,
    menu_choice: Option<MenuCommand>,
    /// Slot width the fake renderer needs; `None` means it cannot measure.
    fit_needs_width: Option<i32>,
    fit_calls: Vec<(PixelSize, u32, u8)>,
}
impl Fake {
    fn new() -> Self {
        Self {
            calls: Vec::new(),
            mono: Duration::from_secs(100),
            now: wall("2026-10-14T12:00:30+00:00"),
            appearance: SystemAppearance::Light,
            font_changed: false,
            shell: shell(10, 1),
            observations: BTreeMap::new(),
            overlays: HashSet::new(),
            paint_result: BTreeMap::new(),
            create_error: false,
            calendar: CalendarOutcome::Sent,
            menu_choice: None,
            fit_needs_width: None,
            fit_calls: Vec::new(),
        }
    }
}
impl Platform for Fake {
    fn monotonic(&mut self) -> Duration {
        self.mono
    }
    fn local_now(&mut self) -> DateTime<FixedOffset> {
        self.now
    }
    fn system_appearance(&mut self) -> SystemAppearance {
        self.appearance
    }
    fn refresh_fonts(&mut self) -> bool {
        std::mem::take(&mut self.font_changed)
    }
    fn shell_identity(&mut self) -> Option<ShellIdentity> {
        self.shell
    }
    fn shell_changed(&mut self, shell: Option<ShellIdentity>) {
        self.calls.push(Call::ShellChanged(shell));
    }
    fn request_discovery(&mut self, generation: ShellGeneration) {
        self.calls.push(Call::RequestDiscovery(generation.0));
    }
    fn note_taskbars(&mut self, _: &[TaskbarSnapshot]) {}
    fn observe(&mut self, taskbar: &TaskbarSnapshot) -> Observation {
        // Unscripted taskbars look exactly as discovered.
        self.observations
            .get(&taskbar.key.id.0)
            .copied()
            .unwrap_or(Observation {
                exists: true,
                visible: true,
                rect: Some(taskbar.taskbar_rect),
                dpi: taskbar.dpi,
                monitor_rect: Some(taskbar.monitor_rect),
                fullscreen: false,
            })
    }
    fn create_overlay(&mut self, key: TaskbarKey, _: PhysicalRect) -> Result<(), String> {
        self.calls.push(Call::Create(key));
        if self.create_error {
            return Err("CreateWindowExW failed".into());
        }
        self.overlays.insert(key);
        Ok(())
    }
    fn set_bounds(&mut self, key: TaskbarKey, _: PhysicalRect) -> Result<(), String> {
        self.calls.push(Call::SetBounds(key));
        Ok(())
    }
    fn destroy_overlay(&mut self, key: TaskbarKey) {
        self.calls.push(Call::Destroy(key));
        self.overlays.remove(&key);
    }
    fn paint(&mut self, key: TaskbarKey, context: &PaintContext<'_>) -> PaintResult {
        self.calls
            .push(Call::Paint(key, context.text.lines.clone()));
        self.paint_result
            .get(&key.id.0)
            .cloned()
            .unwrap_or(PaintResult::Painted)
    }
    fn hide(&mut self, key: TaskbarKey) {
        self.calls.push(Call::Hide(key));
    }
    fn plan_fit(
        &mut self,
        _: &ClockText,
        _: &WorstCase,
        region: PixelSize,
        dpi: u32,
        font_pt: u8,
    ) -> Option<Result<Fit, RenderError>> {
        let needs = self.fit_needs_width?;
        self.fit_calls.push((region, dpi, font_pt));
        let required = PixelSize {
            width: needs,
            height: region.height,
        };
        Some(Ok(if needs > region.width {
            Fit::Overflow(Overflow {
                required,
                available: region,
            })
        } else {
            Fit::Fits(crate::render::Layout {
                font_px: 12.0,
                padding_px: 4,
                region,
                required,
            })
        }))
    }
    fn open_calendar(&mut self, key: TaskbarKey) -> CalendarOutcome {
        self.calls.push(Call::OpenCalendar(key));
        self.calendar.clone()
    }
    fn show_menu(&mut self, anchor: Option<ScreenPoint>, paused: bool) -> Option<MenuCommand> {
        self.calls.push(Call::ShowMenu(anchor, paused));
        self.menu_choice.take()
    }
    fn open_settings(&mut self) {
        self.calls.push(Call::OpenSettings);
    }
    fn restore_tray(&mut self) {
        self.calls.push(Call::RestoreTray);
    }
    fn paused_changed(&mut self, paused: bool) {
        self.calls.push(Call::PausedChanged(paused));
    }
    fn arm_timer(&mut self, delay: Duration) {
        self.calls.push(Call::Timer(delay));
    }
    fn arm_poll(&mut self, delay: Duration) {
        self.calls.push(Call::Poll(delay));
    }
    fn stop(&mut self) {
        self.calls.push(Call::Stop);
    }
}

fn runtime(monitors: Monitors) -> Runtime<Fake> {
    let mut runtime = Runtime::new(Fake::new(), prefs(monitors));
    runtime.start();
    runtime
}
fn report(runtime: &Runtime<Fake>, generation: u64, snapshots: Vec<TaskbarSnapshot>) -> Input {
    Input::Discovery(DiscoveryReport {
        generation: ShellGeneration(generation),
        shell: runtime.shell,
        outcome: Ok(snapshots),
    })
}
fn discover(runtime: &mut Runtime<Fake>, generation: u64, snapshots: Vec<TaskbarSnapshot>) {
    let input = report(runtime, generation, snapshots);
    runtime.handle(input);
}
fn take(runtime: &mut Runtime<Fake>) -> Vec<Call> {
    std::mem::take(&mut runtime.platform.calls)
}
fn advance(runtime: &mut Runtime<Fake>, by: Duration) {
    runtime.platform.mono += by;
    runtime.handle(Input::Poll);
}
fn requests(calls: &[Call]) -> Vec<u64> {
    calls
        .iter()
        .filter_map(|c| match c {
            Call::RequestDiscovery(g) => Some(*g),
            _ => None,
        })
        .collect()
}
fn paints(calls: &[Call]) -> usize {
    calls
        .iter()
        .filter(|c| matches!(c, Call::Paint(..)))
        .count()
}

#[test]
fn start_reads_time_once_requests_discovery_and_arms_both_timers() {
    let mut runtime = Runtime::new(Fake::new(), prefs(Monitors::All));
    runtime.start();
    let calls = take(&mut runtime);
    assert_eq!(
        calls,
        vec![
            Call::ShellChanged(shell(10, 1)),
            Call::RestoreTray,
            // 12:00:30 with a minute cadence: next boundary in 30 s.
            Call::Timer(Duration::from_secs(30)),
            Call::RequestDiscovery(0),
            Call::Poll(RECOVERY_POLL),
        ]
    );
}

#[test]
fn discovery_creates_and_paints_every_selected_monitor_without_restart() {
    let mut runtime = runtime(Monitors::All);
    take(&mut runtime);
    discover(&mut runtime, 0, vec![bar(0, 1, true), bar(0, 2, true)]);
    let calls = take(&mut runtime);
    assert!(calls.contains(&Call::Create(key(0, 1))));
    assert!(calls.contains(&Call::Create(key(0, 2))));
    assert_eq!(paints(&calls), 2);
    assert_eq!(
        runtime.state().overlay_state(key(0, 2)),
        OverlayState::Visible
    );
    let diagnostics = runtime.diagnostics();
    assert!(diagnostics.taskbars.iter().all(|t| t.reason.is_none()));
}

#[test]
fn primary_mode_paints_only_the_primary_and_explains_the_other() {
    let mut runtime = runtime(Monitors::Primary);
    discover(&mut runtime, 0, vec![bar(0, 1, true), bar(0, 2, true)]);
    assert!(runtime.platform.overlays.contains(&key(0, 1)));
    assert!(!runtime.platform.overlays.contains(&key(0, 2)));
    let secondary = &runtime.diagnostics().taskbars[1];
    assert_eq!(
        secondary.state,
        OverlayState::Hidden(clock_core::HiddenReason::Paused)
    );
    assert_eq!(secondary.reason, Some(Reason::NotShown));
}

#[test]
fn timer_takes_one_fresh_snapshot_and_repaints_only_changed_text() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true), bar(0, 2, true)]);
    take(&mut runtime);
    // Same minute: nothing changes visually but the one-shot is re-armed.
    runtime.platform.now = wall("2026-10-14T12:00:59+00:00");
    runtime.handle(Input::Timer);
    let calls = take(&mut runtime);
    assert_eq!(paints(&calls), 0);
    assert_eq!(calls, vec![Call::Timer(Duration::from_secs(1))]);
    runtime.platform.now = wall("2026-10-14T12:01:00+00:00");
    runtime.handle(Input::Timer);
    let calls = take(&mut runtime);
    assert_eq!(paints(&calls), 2);
    assert!(calls.contains(&Call::Timer(Duration::from_secs(60))));
}

#[test]
fn clock_changes_and_resume_take_fresh_time_instead_of_extrapolating() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    take(&mut runtime);
    // Manual change backwards by a day (also covers time zone / DST edits).
    runtime.platform.now = wall("2026-10-13T08:15:00+01:00");
    runtime.handle(Input::Signal(Signal::TimeChanged));
    let calls = take(&mut runtime);
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::Paint(_, lines) if lines[0] == "08:15"))
    );
    assert!(calls.contains(&Call::Timer(Duration::from_secs(60))));
    // Resume: time, then topology (after the settle window), appearance and shell.
    runtime.platform.now = wall("2026-10-14T09:00:00+02:00");
    runtime.handle(Input::Signal(Signal::Resumed));
    let calls = take(&mut runtime);
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::Paint(_, lines) if lines[0] == "09:00"))
    );
    assert!(
        requests(&calls).is_empty(),
        "topology waits for the settle window"
    );
    advance(&mut runtime, TOPOLOGY_SETTLE);
    assert_eq!(requests(&take(&mut runtime)), vec![0]);
}

#[test]
fn topology_bursts_settle_into_one_discovery_and_follow_up_after_an_in_flight_one() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    take(&mut runtime);
    for _ in 0..5 {
        runtime.handle(Input::Signal(Signal::TopologyChanged));
    }
    assert!(requests(&take(&mut runtime)).is_empty());
    advance(&mut runtime, TOPOLOGY_SETTLE);
    assert_eq!(requests(&take(&mut runtime)), vec![0]);
    // Another change while that discovery is running requires a second refresh,
    // because the in-flight answer may predate it.
    runtime.handle(Input::Signal(Signal::TopologyChanged));
    discover(&mut runtime, 0, vec![bar(0, 1, true), bar(0, 2, true)]);
    advance(&mut runtime, TOPOLOGY_SETTLE);
    assert_eq!(requests(&take(&mut runtime)), vec![0]);
    assert!(runtime.platform.overlays.contains(&key(0, 2)));
}

#[test]
fn taskbar_created_replaces_the_generation_and_rejects_old_discovery_results() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true), bar(0, 2, true)]);
    take(&mut runtime);
    runtime.platform.shell = shell(77, 5);
    runtime.handle(Input::Signal(Signal::TaskbarCreated));
    let calls = take(&mut runtime);
    assert!(calls.contains(&Call::Destroy(key(0, 1))));
    assert!(calls.contains(&Call::Destroy(key(0, 2))));
    assert_eq!(requests(&calls), vec![1]);
    assert!(calls.contains(&Call::ShellChanged(shell(77, 5))));
    assert!(runtime.platform.overlays.is_empty());
    // A late result from the old Explorer must not resurrect anything.
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    assert!(
        take(&mut runtime)
            .iter()
            .all(|c| !matches!(c, Call::Create(_)))
    );
    assert!(runtime.platform.overlays.is_empty());
    runtime.platform.shell = shell(77, 5);
    discover(&mut runtime, 1, vec![bar(1, 1, true), bar(1, 2, true)]);
    assert!(runtime.platform.overlays.contains(&key(1, 1)));
    assert!(runtime.platform.overlays.contains(&key(1, 2)));
}

#[test]
fn a_reused_pid_with_a_new_creation_time_is_a_new_shell() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    take(&mut runtime);
    runtime.platform.shell = shell(10, 999);
    advance(&mut runtime, RECOVERY_POLL);
    let calls = take(&mut runtime);
    assert!(calls.contains(&Call::Destroy(key(0, 1))));
    assert_eq!(requests(&calls), vec![1]);
}

#[test]
fn a_missing_explorer_removes_overlays_and_recovers_when_it_returns() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    take(&mut runtime);
    runtime.platform.shell = None;
    advance(&mut runtime, RECOVERY_POLL);
    let calls = take(&mut runtime);
    assert!(calls.contains(&Call::Destroy(key(0, 1))));
    assert_eq!(requests(&calls), vec![1]);
    // No Explorer: an empty batch is legitimate and not retried as a failure.
    discover(&mut runtime, 1, vec![]);
    assert_eq!(runtime.diagnostics().consecutive_failures, 0);
    runtime.platform.shell = shell(88, 9);
    advance(&mut runtime, RECOVERY_POLL);
    assert_eq!(requests(&take(&mut runtime)), vec![2]);
    discover(&mut runtime, 2, vec![bar(2, 1, true)]);
    assert!(runtime.platform.overlays.contains(&key(2, 1)));
}

#[test]
fn a_result_for_a_replaced_shell_is_rejected_even_if_the_generation_matches() {
    let mut runtime = runtime(Monitors::All);
    take(&mut runtime);
    runtime.platform.shell = shell(10, 1);
    let input = Input::Discovery(DiscoveryReport {
        generation: ShellGeneration(0),
        shell: shell(4, 4), // helper saw some other Explorer
        outcome: Ok(vec![bar(0, 1, true)]),
    });
    runtime.handle(input);
    assert!(runtime.platform.overlays.is_empty());
    assert!(matches!(
        runtime.diagnostics().last_discovery_failure,
        Some(DiscoveryFailure::Rejected(_))
    ));
}

#[test]
fn failed_discovery_fails_closed_then_retries_with_bounded_backoff() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    // Topology refresh whose helper hits the deadline.
    runtime.handle(Input::Signal(Signal::TopologyChanged));
    advance(&mut runtime, TOPOLOGY_SETTLE);
    take(&mut runtime);
    let fail = |runtime: &mut Runtime<Fake>, failure| {
        let generation = runtime.state().generation().0;
        runtime.handle(Input::Discovery(DiscoveryReport {
            generation: ShellGeneration(generation),
            shell: runtime.shell,
            outcome: Err(failure),
        }));
    };
    fail(&mut runtime, DiscoveryFailure::Deadline);
    let calls = take(&mut runtime);
    assert!(
        calls.contains(&Call::Destroy(key(0, 1))),
        "partial output is discarded"
    );
    assert!(runtime.platform.overlays.is_empty());
    // First retry after 2 s, then 4 s, 8 s ... capped at 60 s.
    let mut waits = Vec::new();
    for _ in 0..8 {
        let before = runtime.platform.mono;
        loop {
            advance(&mut runtime, Duration::from_millis(500));
            if !requests(&take(&mut runtime)).is_empty() {
                break;
            }
        }
        waits.push((runtime.platform.mono - before).as_secs_f32());
        fail(&mut runtime, DiscoveryFailure::HelperExit(Some(1)));
        take(&mut runtime);
    }
    assert_eq!(runtime.diagnostics().consecutive_failures, 9);
    assert!(waits[0] >= 2.0 && waits[0] < 2.6, "{waits:?}");
    assert!(waits[1] >= 4.0 && waits[1] < 4.6, "{waits:?}");
    assert!(waits[2] >= 8.0 && waits[2] < 8.6, "{waits:?}");
    assert!(waits.iter().all(|w| *w <= 60.6), "{waits:?}");
    assert!(waits[7] >= 60.0, "backoff reaches its cap: {waits:?}");
    // Recovery clears the failure state and restores the overlay.
    let generation = runtime.state().generation().0;
    loop {
        advance(&mut runtime, Duration::from_millis(500));
        if !requests(&take(&mut runtime)).is_empty() {
            break;
        }
    }
    discover(&mut runtime, generation, vec![bar(generation, 1, true)]);
    assert_eq!(runtime.diagnostics().consecutive_failures, 0);
    assert!(runtime.platform.overlays.contains(&key(generation, 1)));
}

#[test]
fn a_failed_helper_teardown_disables_discovery_without_further_requests() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    take(&mut runtime);
    runtime.handle(Input::Signal(Signal::TopologyChanged));
    advance(&mut runtime, TOPOLOGY_SETTLE);
    take(&mut runtime);
    runtime.handle(Input::Discovery(DiscoveryReport {
        generation: ShellGeneration(0),
        shell: runtime.shell,
        outcome: Err(DiscoveryFailure::TeardownFailed),
    }));
    assert!(runtime.diagnostics().discovery_disabled);
    assert!(
        runtime.platform.overlays.is_empty(),
        "native clock restored"
    );
    take(&mut runtime);
    // No signal or poll can start another helper, but Explorer loss still
    // resolves the reducer without one.
    runtime.handle(Input::Signal(Signal::TopologyChanged));
    for _ in 0..10 {
        advance(&mut runtime, Duration::from_secs(30));
    }
    runtime.platform.shell = shell(5, 5);
    runtime.handle(Input::Signal(Signal::TaskbarCreated));
    assert!(requests(&take(&mut runtime)).is_empty());
}

#[test]
fn teardown_failure_for_a_superseded_generation_still_releases_the_reducer() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    runtime.platform.shell = shell(5, 5);
    runtime.handle(Input::Signal(Signal::TaskbarCreated)); // generation 1 requested
    take(&mut runtime);
    // The worker died while reaping generation 0's helper.
    runtime.handle(Input::Discovery(DiscoveryReport {
        generation: ShellGeneration(0),
        shell: None,
        outcome: Err(DiscoveryFailure::TeardownFailed),
    }));
    assert!(runtime.diagnostics().discovery_disabled);
    // The outstanding generation-1 request was answered locally: a later
    // Explorer loss therefore starts no helper and still completes cleanly.
    runtime.platform.shell = shell(6, 6);
    runtime.handle(Input::Signal(Signal::TaskbarCreated));
    assert!(requests(&take(&mut runtime)).is_empty());
    assert!(runtime.platform.overlays.is_empty());
}

#[test]
fn an_empty_batch_with_a_running_shell_is_retried_not_trusted() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![]);
    assert_eq!(
        runtime.diagnostics().last_discovery_failure,
        Some(DiscoveryFailure::NoTaskbar)
    );
    take(&mut runtime);
    advance(&mut runtime, RECOVERY_POLL);
    assert_eq!(requests(&take(&mut runtime)), vec![0]);
}

#[test]
fn auto_hide_hides_and_restores_each_monitor_independently() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true), bar(0, 2, true)]);
    take(&mut runtime);
    let sliding = Observation {
        rect: Some(rect(0, 1070, 1920, 48)),
        ..runtime.platform.observe(&bar(0, 1, true))
    };
    runtime.platform.observations.insert(1, sliding);
    runtime.handle(Input::Signal(Signal::Shell(ShellEvent::TaskbarChanged)));
    let calls = take(&mut runtime);
    assert_eq!(
        calls,
        vec![
            Call::Hide(key(0, 1)),
            Call::Poll(crate::events::TRANSITION_POLL)
        ]
    );
    assert_eq!(
        runtime.state().overlay_state(key(0, 2)),
        OverlayState::Visible
    );
    // The slide finishes: shown again, which needs a repaint of the hidden window.
    runtime.platform.observations.remove(&1);
    advance(&mut runtime, Duration::from_millis(100));
    let calls = take(&mut runtime);
    assert_eq!(paints(&calls), 1);
    assert!(!calls.iter().any(|c| matches!(c, Call::Create(_))));
}

#[test]
fn transition_polling_is_short_and_bounded() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    take(&mut runtime);
    runtime.handle(Input::Signal(Signal::Shell(ShellEvent::TaskbarChanged)));
    assert!(take(&mut runtime).contains(&Call::Poll(crate::events::TRANSITION_POLL)));
    // Nothing changes: after two stable polls the cadence relaxes to two seconds.
    let mut delays = Vec::new();
    for _ in 0..4 {
        advance(&mut runtime, Duration::from_millis(100));
        delays.extend(take(&mut runtime).into_iter().filter_map(|c| match c {
            Call::Poll(d) => Some(d),
            _ => None,
        }));
    }
    assert_eq!(*delays.last().unwrap(), RECOVERY_POLL, "{delays:?}");
    assert!(delays.contains(&crate::events::TRANSITION_POLL));
}

#[test]
fn clock_ticks_cannot_postpone_the_recovery_poll() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    take(&mut runtime);
    for second in 1..30 {
        runtime.platform.now = wall(&format!("2026-10-14T12:00:{second:02}+00:00"));
        runtime.handle(Input::Timer);
    }
    assert!(
        take(&mut runtime)
            .iter()
            .all(|c| !matches!(c, Call::Poll(_))),
        "the armed poll timer is reused"
    );
}

#[test]
fn a_moved_or_resized_taskbar_triggers_one_rediscovery_not_a_loop() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    take(&mut runtime);
    let moved = Observation {
        rect: Some(rect(0, 1000, 1920, 80)),
        ..runtime.platform.observe(&bar(0, 1, true))
    };
    runtime.platform.observations.insert(1, moved);
    advance(&mut runtime, RECOVERY_POLL);
    advance(&mut runtime, TOPOLOGY_SETTLE);
    assert_eq!(requests(&take(&mut runtime)), vec![0]);
    // Discovery returns the same snapshot (the shell still reports the odd rect):
    // the same rectangle must not cause another request.
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    for _ in 0..5 {
        advance(&mut runtime, RECOVERY_POLL);
    }
    assert!(requests(&take(&mut runtime)).is_empty());
    // A *different* odd rectangle is a new fact.
    let again = Observation {
        rect: Some(rect(0, 990, 1920, 90)),
        ..moved
    };
    runtime.platform.observations.insert(1, again);
    advance(&mut runtime, RECOVERY_POLL);
    advance(&mut runtime, TOPOLOGY_SETTLE);
    assert_eq!(requests(&take(&mut runtime)), vec![0]);
}

#[test]
fn a_dpi_change_or_unplugged_monitor_rediscovers() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true), bar(0, 2, true)]);
    take(&mut runtime);
    let unplugged = Observation::GONE;
    runtime.platform.observations.insert(2, unplugged);
    advance(&mut runtime, RECOVERY_POLL);
    advance(&mut runtime, TOPOLOGY_SETTLE);
    assert_eq!(requests(&take(&mut runtime)), vec![0]);
    // The new batch drops the removed monitor; its overlay is destroyed.
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    assert!(take(&mut runtime).contains(&Call::Destroy(key(0, 2))));
    assert!(!runtime.platform.overlays.contains(&key(0, 2)));
    // Primary swap: the other taskbar becomes primary under "primary only".
    runtime.handle(Input::Preferences(prefs(Monitors::Primary)));
    let mut swapped_secondary = bar(0, 2, true);
    swapped_secondary.primary = true;
    let mut former_primary = bar(0, 1, true);
    former_primary.primary = false;
    runtime.handle(Input::Signal(Signal::TopologyChanged));
    advance(&mut runtime, TOPOLOGY_SETTLE);
    take(&mut runtime);
    discover(&mut runtime, 0, vec![former_primary, swapped_secondary]);
    assert_eq!(
        runtime.state().overlay_state(key(0, 2)),
        OverlayState::Visible
    );
    assert_eq!(
        runtime.state().overlay_state(key(0, 1)),
        OverlayState::Hidden(clock_core::HiddenReason::Paused)
    );
}

#[test]
fn fullscreen_hides_only_the_covered_monitor_and_restores_it() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true), bar(0, 2, true)]);
    take(&mut runtime);
    let covered = Observation {
        fullscreen: true,
        ..runtime.platform.observe(&bar(0, 2, true))
    };
    runtime.platform.observations.insert(2, covered);
    runtime.handle(Input::Signal(Signal::Shell(ShellEvent::ForegroundChanged)));
    assert_eq!(take(&mut runtime), vec![Call::Hide(key(0, 2))]);
    assert_eq!(
        runtime.state().overlay_state(key(0, 1)),
        OverlayState::Visible
    );
    runtime.platform.observations.remove(&2);
    runtime.handle(Input::Signal(Signal::Shell(ShellEvent::ForegroundChanged)));
    assert_eq!(paints(&take(&mut runtime)), 1);
}

#[test]
fn a_new_taskbar_that_starts_under_a_fullscreen_window_is_hidden_immediately() {
    let mut runtime = runtime(Monitors::All);
    runtime.platform.observations.insert(
        1,
        Observation {
            fullscreen: true,
            exists: true,
            visible: true,
            rect: Some(bar(0, 1, true).taskbar_rect),
            dpi: 96,
            monitor_rect: Some(bar(0, 1, true).monitor_rect),
        },
    );
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    assert_eq!(
        runtime.state().overlay_state(key(0, 1)),
        OverlayState::Hidden(clock_core::HiddenReason::Fullscreen)
    );
}

#[test]
fn lock_hides_and_unlock_refreshes_time_and_discovery_before_showing() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    take(&mut runtime);
    runtime.handle(Input::Signal(Signal::SessionLocked));
    assert_eq!(take(&mut runtime), vec![Call::Hide(key(0, 1))]);
    runtime.platform.now = wall("2026-10-14T18:30:00+00:00");
    runtime.handle(Input::Signal(Signal::SessionUnlocked));
    let calls = take(&mut runtime);
    assert_eq!(requests(&calls), vec![0]);
    assert_eq!(paints(&calls), 0, "no pre-lock geometry is shown");
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    let calls = take(&mut runtime);
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::Paint(_, lines) if lines[0] == "18:30"))
    );
}

#[test]
fn appearance_changes_repaint_live_and_unrelated_setting_changes_do_not() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    take(&mut runtime);
    runtime.handle(Input::Signal(Signal::AppearanceMaybeChanged));
    assert_eq!(paints(&take(&mut runtime)), 0, "same appearance");
    runtime.platform.appearance = SystemAppearance::Dark;
    runtime.handle(Input::Signal(Signal::AppearanceMaybeChanged));
    assert_eq!(paints(&take(&mut runtime)), 1);
    runtime.platform.font_changed = true;
    runtime.handle(Input::Signal(Signal::AppearanceMaybeChanged));
    assert_eq!(paints(&take(&mut runtime)), 1, "system font change");
}

#[test]
fn preference_changes_apply_to_the_running_clock() {
    let mut runtime = runtime(Monitors::Primary);
    discover(&mut runtime, 0, vec![bar(0, 1, true), bar(0, 2, true)]);
    take(&mut runtime);
    let format = Preferences {
        format: "%Y".into(),
        monitors: Monitors::All,
        theme: Theme::Dark,
        ..Preferences::default()
    }
    .validate()
    .unwrap();
    runtime.handle(Input::Preferences(format));
    let calls = take(&mut runtime);
    assert!(calls.contains(&Call::Create(key(0, 2))), "all monitors");
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::Paint(_, lines) if lines == &vec!["2026".to_string()]))
    );
    assert_eq!(runtime.worst_case.fingerprint(), {
        WorstCase::for_format(runtime.state().preferences().format()).fingerprint()
    });
}

#[test]
fn overflow_and_render_failure_hide_only_the_affected_overlay_with_a_reason() {
    let mut runtime = runtime(Monitors::All);
    let overflow = Overflow {
        required: PixelSize {
            width: 140,
            height: 40,
        },
        available: PixelSize {
            width: 100,
            height: 48,
        },
    };
    runtime
        .platform
        .paint_result
        .insert(2, PaintResult::Overflow(overflow));
    runtime
        .platform
        .paint_result
        .insert(3, PaintResult::Failed("device lost".into()));
    let mut third = bar(0, 1, true);
    third.key.id = TaskbarId(3);
    third.primary = false;
    discover(
        &mut runtime,
        0,
        vec![bar(0, 1, true), bar(0, 2, true), third],
    );
    let calls = take(&mut runtime);
    assert!(calls.contains(&Call::Hide(key(0, 2))));
    assert!(calls.contains(&Call::Hide(key(0, 3))));
    assert!(!calls.contains(&Call::Hide(key(0, 1))));
    let diagnostics = runtime.diagnostics();
    assert_eq!(diagnostics.taskbars[0].reason, None);
    assert_eq!(
        diagnostics.taskbars[1].reason,
        Some(Reason::Overflow(overflow))
    );
    assert_eq!(
        diagnostics.taskbars[2].reason,
        Some(Reason::RenderFailed("device lost".into()))
    );
    // Fixing the cause (shorter text) clears the reason on the next paint.
    runtime.platform.paint_result.clear();
    runtime.handle(Input::Preferences(
        Preferences {
            format: "%H".into(),
            monitors: Monitors::All,
            ..Preferences::default()
        }
        .validate()
        .unwrap(),
    ));
    assert!(
        runtime
            .diagnostics()
            .taskbars
            .iter()
            .all(|t| t.reason.is_none())
    );
}

#[test]
fn overlay_creation_failure_is_reported_and_hides_the_overlay() {
    let mut runtime = runtime(Monitors::All);
    runtime.platform.create_error = true;
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    assert_eq!(
        runtime.diagnostics().taskbars[0].reason,
        Some(Reason::PlacementFailed("CreateWindowExW failed".into()))
    );
}

#[test]
fn quit_destroys_overlays_stops_the_platform_once_and_ignores_late_input() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true), bar(0, 2, true)]);
    take(&mut runtime);
    runtime.handle(Input::Quit);
    let calls = take(&mut runtime);
    assert!(calls.contains(&Call::Destroy(key(0, 1))));
    assert!(calls.contains(&Call::Destroy(key(0, 2))));
    assert_eq!(calls.iter().filter(|c| **c == Call::Stop).count(), 1);
    assert!(runtime.is_finished());
    // Late timers, polls, signals and results are inert: no resurrected overlay.
    runtime.handle(Input::Timer);
    runtime.handle(Input::Poll);
    runtime.handle(Input::Signal(Signal::TaskbarCreated));
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    runtime.handle(Input::Quit);
    assert!(take(&mut runtime).is_empty());
    assert!(runtime.platform.overlays.is_empty());
}

#[test]
fn end_session_quits_like_exit() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    take(&mut runtime);
    runtime.handle(Input::Signal(Signal::EndSession));
    assert!(runtime.is_finished());
    assert!(runtime.platform.overlays.is_empty());
}

#[test]
fn repeated_explorer_failures_never_accumulate_overlays_or_requests() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true), bar(0, 2, true)]);
    let mut live = HashSet::new();
    for round in 1..=50u64 {
        runtime.platform.shell = shell(100 + round as u32, round);
        runtime.handle(Input::Signal(Signal::TaskbarCreated));
        // The previous generation's helper reports late, then the new one answers.
        discover(&mut runtime, round - 1, vec![bar(round - 1, 1, true)]);
        discover(
            &mut runtime,
            round,
            vec![bar(round, 1, true), bar(round, 2, true)],
        );
        live = runtime.platform.overlays.clone();
        assert_eq!(live.len(), 2, "round {round}");
        assert!(live.iter().all(|k| k.generation.0 == round));
    }
    assert_eq!(live.len(), 2);
}

// ---- R09: clock actions, menu, pause and tray ------------------------------

fn shown(monitors: Monitors) -> Runtime<Fake> {
    let mut runtime = runtime(monitors);
    discover(&mut runtime, 0, vec![bar(0, 1, true), bar(0, 2, true)]);
    take(&mut runtime);
    runtime
}
fn click(runtime: &mut Runtime<Fake>, id: u64) {
    runtime.handle(Input::Action(UserAction::Activate(key(0, id))));
}
fn opens(calls: &[Call]) -> usize {
    calls
        .iter()
        .filter(|c| matches!(c, Call::OpenCalendar(_)))
        .count()
}

#[test]
fn one_click_sends_one_calendar_action_for_the_clicked_taskbar() {
    let mut runtime = shown(Monitors::All);
    click(&mut runtime, 2);
    assert_eq!(take(&mut runtime), vec![Call::OpenCalendar(key(0, 2))]);
}

#[test]
fn rapid_repeat_clicks_coalesce_but_a_later_deliberate_click_works() {
    let mut runtime = shown(Monitors::All);
    click(&mut runtime, 1);
    click(&mut runtime, 1);
    click(&mut runtime, 2);
    assert_eq!(opens(&take(&mut runtime)), 1, "double click is one action");
    runtime.platform.mono += ACTIVATION_COOLDOWN;
    click(&mut runtime, 1);
    assert_eq!(opens(&take(&mut runtime)), 1);
}

#[test]
fn a_click_that_is_stale_hidden_or_unknown_does_nothing() {
    let mut runtime = shown(Monitors::Primary);
    // Not shown by design (other monitor), and a key from another generation.
    click(&mut runtime, 2);
    runtime.handle(Input::Action(UserAction::Activate(key(7, 1))));
    assert_eq!(opens(&take(&mut runtime)), 0);

    // Fullscreen hides the overlay; a click queued before that is ignored.
    let mut runtime = shown(Monitors::All);
    runtime.platform.observations.insert(
        1,
        Observation {
            fullscreen: true,
            ..Observation {
                exists: true,
                visible: true,
                rect: Some(bar(0, 1, true).taskbar_rect),
                dpi: 96,
                monitor_rect: Some(bar(0, 1, true).monitor_rect),
                fullscreen: false,
            }
        },
    );
    runtime.handle(Input::Poll);
    take(&mut runtime);
    click(&mut runtime, 1);
    assert_eq!(opens(&take(&mut runtime)), 0);
}

#[test]
fn held_modifiers_send_nothing_and_do_not_consume_the_cooldown_or_remove_overlays() {
    let mut runtime = shown(Monitors::All);
    runtime.platform.calendar = CalendarOutcome::ModifiersHeld;
    click(&mut runtime, 1);
    // The user lets go: the very next click works immediately.
    runtime.platform.calendar = CalendarOutcome::Sent;
    click(&mut runtime, 1);
    let calls = take(&mut runtime);
    assert_eq!(opens(&calls), 2);
    assert!(!calls.iter().any(|c| matches!(c, Call::Hide(_))));
    assert!(
        runtime
            .diagnostics()
            .taskbars
            .iter()
            .all(|t| t.reason.is_none())
    );
}

#[test]
fn an_unknown_calendar_failure_removes_every_overlay_records_why_and_never_retries() {
    let mut runtime = shown(Monitors::All);
    runtime.platform.calendar = CalendarOutcome::Failed("input blocked".into());
    click(&mut runtime, 1);
    let calls = take(&mut runtime);
    assert_eq!(opens(&calls), 1, "no retry of the failed action");
    assert!(calls.contains(&Call::Hide(key(0, 1))));
    assert!(calls.contains(&Call::Hide(key(0, 2))));
    for taskbar in runtime.diagnostics().taskbars {
        assert_eq!(
            taskbar.reason,
            Some(Reason::ActionFailed("input blocked".into()))
        );
    }
    // Further clicks on a removed overlay do nothing, and ticks do not bring it back.
    click(&mut runtime, 1);
    runtime.platform.now = wall("2026-10-14T12:05:00+00:00");
    runtime.handle(Input::Timer);
    let calls = take(&mut runtime);
    assert_eq!(opens(&calls), 0);
    assert_eq!(paints(&calls), 0);
}

#[test]
fn pause_then_resume_gives_a_failed_overlay_another_chance() {
    let mut runtime = shown(Monitors::All);
    runtime.platform.calendar = CalendarOutcome::Failed("x".into());
    click(&mut runtime, 1);
    take(&mut runtime);
    runtime.platform.menu_choice = Some(MenuCommand::TogglePause);
    runtime.handle(Input::Action(UserAction::ShowMenu(None)));
    assert!(runtime.diagnostics().paused);
    runtime.platform.mono += ACTIVATION_COOLDOWN;
    runtime.platform.menu_choice = Some(MenuCommand::TogglePause);
    runtime.handle(Input::Action(UserAction::ShowMenu(None)));
    let calls = take(&mut runtime);
    assert_eq!(paints(&calls), 2, "resume repaints both clocks");
    assert!(
        runtime
            .diagnostics()
            .taskbars
            .iter()
            .all(|t| t.reason.is_none())
    );
}

#[test]
fn the_menu_reports_the_current_mode_and_dispatches_exactly_the_chosen_command() {
    let mut runtime = shown(Monitors::All);
    let at = Some(ScreenPoint { x: -100, y: 1040 });
    runtime.handle(Input::Action(UserAction::ShowMenu(at)));
    assert_eq!(take(&mut runtime), vec![Call::ShowMenu(at, false)]);

    runtime.platform.mono += ACTIVATION_COOLDOWN;
    runtime.platform.menu_choice = Some(MenuCommand::Settings);
    runtime.handle(Input::Action(UserAction::ShowMenu(at)));
    assert_eq!(
        take(&mut runtime),
        vec![Call::ShowMenu(at, false), Call::OpenSettings]
    );
    assert!(!runtime.is_finished());
}

#[test]
fn pause_hides_every_overlay_so_the_native_clock_shows_and_resume_restores_them() {
    let mut runtime = shown(Monitors::All);
    runtime.platform.menu_choice = Some(MenuCommand::TogglePause);
    runtime.handle(Input::Action(UserAction::ShowMenu(None)));
    let calls = take(&mut runtime);
    assert!(calls.contains(&Call::Hide(key(0, 1))));
    assert!(calls.contains(&Call::Hide(key(0, 2))));
    assert!(calls.contains(&Call::PausedChanged(true)));
    let diagnostics = runtime.diagnostics();
    assert!(diagnostics.paused);
    assert!(diagnostics.taskbars.iter().all(|t| matches!(
        t.state,
        OverlayState::Hidden(clock_core::HiddenReason::Paused)
    )));

    // Ticks, rediscovery and a click while paused never show or act.
    runtime.platform.now = wall("2026-10-14T12:01:00+00:00");
    runtime.handle(Input::Timer);
    click(&mut runtime, 1);
    let calls = take(&mut runtime);
    assert_eq!(paints(&calls) + opens(&calls), 0);

    // The menu now offers Resume.
    runtime.platform.mono += ACTIVATION_COOLDOWN;
    runtime.platform.menu_choice = Some(MenuCommand::TogglePause);
    runtime.handle(Input::Action(UserAction::ShowMenu(None)));
    let calls = take(&mut runtime);
    assert_eq!(calls[0], Call::ShowMenu(None, true));
    assert_eq!(paints(&calls), 2);
    assert!(calls.contains(&Call::PausedChanged(false)));
    assert!(!runtime.diagnostics().paused);
}

#[test]
fn exit_from_the_menu_quits_like_any_other_exit() {
    let mut runtime = shown(Monitors::All);
    runtime.platform.menu_choice = Some(MenuCommand::Exit);
    runtime.handle(Input::Action(UserAction::ShowMenu(None)));
    let calls = take(&mut runtime);
    assert!(calls.contains(&Call::Destroy(key(0, 1))));
    assert_eq!(calls.iter().filter(|c| **c == Call::Stop).count(), 1);
    assert!(runtime.is_finished());
    runtime.handle(Input::Action(UserAction::ShowMenu(None)));
    assert!(take(&mut runtime).is_empty());
}

#[test]
fn mouse_and_keyboard_tray_events_for_one_gesture_open_one_menu() {
    let mut runtime = shown(Monitors::All);
    // NIN_SELECT and NIN_KEYSELECT can both arrive for one activation.
    runtime.handle(Input::Action(UserAction::ShowMenu(None)));
    runtime.handle(Input::Action(UserAction::ShowMenu(None)));
    let menus = take(&mut runtime)
        .iter()
        .filter(|c| matches!(c, Call::ShowMenu(..)))
        .count();
    assert_eq!(menus, 1);
}

#[test]
fn explorer_restart_re_registers_the_tray_from_the_existing_recovery_path_only() {
    let mut runtime = shown(Monitors::All);
    runtime.handle(Input::Signal(Signal::TaskbarCreated));
    let calls = take(&mut runtime);
    assert_eq!(calls.iter().filter(|c| **c == Call::RestoreTray).count(), 1);

    // A new Explorer process noticed by the poll is the same single path.
    runtime.platform.shell = shell(11, 2);
    advance(&mut runtime, RECOVERY_POLL);
    let calls = take(&mut runtime);
    assert_eq!(calls.iter().filter(|c| **c == Call::RestoreTray).count(), 1);

    // Steady state never touches the tray: no second recovery loop.
    for _ in 0..5 {
        advance(&mut runtime, RECOVERY_POLL);
    }
    assert!(!take(&mut runtime).contains(&Call::RestoreTray));
}

#[test]
fn pause_survives_explorer_restart() {
    let mut runtime = shown(Monitors::All);
    runtime.platform.menu_choice = Some(MenuCommand::TogglePause);
    runtime.handle(Input::Action(UserAction::ShowMenu(None)));
    runtime.handle(Input::Signal(Signal::TaskbarCreated));
    discover(&mut runtime, 1, vec![bar(1, 1, true), bar(1, 2, true)]);
    assert!(runtime.diagnostics().paused);
    assert!(
        runtime
            .platform
            .overlays
            .iter()
            .all(|k| k.generation.0 == 0)
    );
}

// ---- Settings fit checks (R10) -------------------------------------------

fn fit_format() -> CompiledFormat {
    CompiledFormat::parse("%H:%M\n%g%V.%u").unwrap()
}

#[test]
fn settings_fit_check_measures_only_selected_taskbars_with_a_known_clock_slot() {
    let mut runtime = runtime(Monitors::All);
    // id 3 has no proven clock rectangle, so there is nothing to measure there.
    let mut unknown = bar(0, 2, false);
    unknown.key = key(0, 3);
    discover(
        &mut runtime,
        0,
        vec![bar(0, 1, true), bar(0, 2, true), unknown],
    );
    runtime.platform.fit_needs_width = Some(150);
    let now = wall("2026-10-13T09:15:00+02:00");

    let primary = runtime.check(&fit_format(), 9, Monitors::Primary, now);
    assert_eq!(primary.len(), 1);
    assert!(primary[0].primary);
    assert_eq!(primary[0].overflow.required.width, 150);
    assert_eq!(primary[0].overflow.available.width, 100);

    let all = runtime.check(&fit_format(), 14, Monitors::All, now);
    assert_eq!(
        all.iter().map(|p| p.primary).collect::<Vec<_>>(),
        [true, false]
    );
    // Each taskbar is measured at its own DPI and with the draft's font size.
    let calls = &runtime.platform.fit_calls;
    assert_eq!(
        calls[1..],
        [
            (
                PixelSize {
                    width: 100,
                    height: 48
                },
                96,
                14
            ),
            (
                PixelSize {
                    width: 100,
                    height: 48
                },
                144,
                14
            )
        ]
    );
}

#[test]
fn settings_fit_check_reports_nothing_when_it_fits_or_cannot_measure() {
    let mut runtime = runtime(Monitors::All);
    discover(&mut runtime, 0, vec![bar(0, 1, true)]);
    let now = wall("2026-10-13T09:15:00+02:00");
    // A platform that cannot measure never blocks Apply.
    assert!(
        runtime
            .check(&fit_format(), 9, Monitors::All, now)
            .is_empty()
    );
    runtime.platform.fit_needs_width = Some(100);
    assert!(
        runtime
            .check(&fit_format(), 9, Monitors::All, now)
            .is_empty()
    );
    // Before discovery there are no slots at all.
    let mut fresh = Runtime::new(Fake::new(), prefs(Monitors::All));
    fresh.platform.fit_needs_width = Some(1_000);
    assert!(fresh.check(&fit_format(), 9, Monitors::All, now).is_empty());
    assert!(fresh.platform.fit_calls.is_empty());
}
