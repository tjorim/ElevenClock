//! Event-driven runtime coordinator (R08).
//!
//! `Runtime<P>` is the one place that connects R06's pure reducer to the world:
//! it turns platform [`Signal`]s, timers and discovery reports into core
//! [`Event`]s, then applies the resulting [`Effect`]s through a [`Platform`].
//! There is no second state machine: visibility, fullscreen, lock, theme,
//! preference and generation decisions all stay in `clock_core::reduce`. What
//! lives here is the *lifecycle* the reducer deliberately leaves to the runtime
//! (see `docs/design/state-and-scheduling.md`):
//!
//! * discovery is requested once per generation, retried with bounded backoff
//!   after a failure, and re-run if topology changed while it was in flight;
//! * a failed, late or stale discovery fails closed (an authoritative empty batch
//!   removes overlays) instead of leaving the reducer waiting forever;
//! * Explorer replacement is detected by `TaskbarCreated` *and* by shell process
//!   identity (PID + creation time), and old-generation results are discarded;
//! * auto-hide, relocation and fullscreen are observed from events and a slow
//!   recovery poll, with short bounded polling while a taskbar is animating;
//! * user actions (R09) arrive as [`Input::Action`]: one calendar action per
//!   click, the fixed menu, Pause/Resume (a core event, so the reducer hides every
//!   overlay), Settings and Exit. The tray icon is re-registered from the same
//!   shell-replacement path as everything else, not from a second recovery loop.
//!
//! The coordinator is generic over [`Platform`] so every rule above is covered by
//! deterministic tests on any OS. The Windows implementation is `native`.
use crate::{
    events::{RECOVERY_POLL, ShellEvent, Signal, TransitionPoll},
    interaction::{ActivationGate, CalendarOutcome, MenuCommand, ScreenPoint},
    logging,
    render::{Fit, Overflow, PixelSize, RenderError, WorstCase},
    settings_model::{FitProblem, LayoutCheck},
    support,
    theme::{AppearanceTracker, SystemAppearance},
};
use chrono::{DateTime, FixedOffset};
use clock_core::{
    ClockText, CompiledFormat, DiscoveryStrategy, Effect, Event, Monitors, OverlayState,
    PhysicalRect, ShellGeneration, State, TaskbarKey, TaskbarSnapshot, TaskbarVisibility,
    ValidatedPreferences, reduce,
};
use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    time::Duration,
};

pub mod shell;
pub use shell::{Observation, Placement};

#[cfg(windows)]
pub mod native;
#[cfg(windows)]
pub(crate) mod overlay_window;

/// Trailing-edge settle window: display changes arrive as several messages, and
/// each discovery costs a helper process, so one refresh serves the whole burst.
pub const TOPOLOGY_SETTLE: Duration = Duration::from_millis(250);
/// Longest wait between discovery retries after repeated failures.
pub const RETRY_CAP: Duration = Duration::from_secs(60);

/// Explorer process identity. A PID alone can be reused, so creation time is part
/// of the identity, exactly as in R02/R03's native epoch checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellIdentity {
    pub pid: u32,
    pub started: u64,
}

/// Why a discovery attempt produced no usable batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoveryFailure {
    /// The five-second deadline passed; partial output was discarded.
    Deadline,
    HelperExit(Option<i32>),
    /// Output was oversized, malformed, or for another generation.
    Rejected(String),
    /// The helper could not be started or isolated.
    Unavailable(String),
    /// Explorer is running but exposed no taskbar (yet): retry, do not trust.
    NoTaskbar,
    /// Helper exit could not be observed. Fatal: no replacement may be started.
    TeardownFailed,
}
impl fmt::Display for DiscoveryFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deadline => f.write_str("Taskbar discovery did not finish in time."),
            Self::HelperExit(code) => {
                write!(f, "Taskbar discovery stopped unexpectedly ({code:?}).")
            }
            Self::Rejected(why) => write!(f, "Taskbar discovery returned unusable data: {why}"),
            Self::Unavailable(why) => write!(f, "Taskbar discovery could not start: {why}"),
            Self::NoTaskbar => f.write_str("Windows did not report a taskbar yet."),
            Self::TeardownFailed => {
                f.write_str("Taskbar discovery was turned off after a helper failed to stop.")
            }
        }
    }
}

/// A completed discovery request, delivered to the UI thread.
#[derive(Clone, Debug)]
pub struct DiscoveryReport {
    pub generation: ShellGeneration,
    /// Shell identity the helper observed; must still be current when delivered.
    pub shell: Option<ShellIdentity>,
    pub outcome: Result<Vec<TaskbarSnapshot>, DiscoveryFailure>,
}

pub struct PaintContext<'a> {
    pub text: &'a ClockText,
    pub preferences: &'a ValidatedPreferences,
    pub worst_case: &'a WorstCase,
    pub appearance: SystemAppearance,
}
#[derive(Clone, Debug, PartialEq)]
pub enum PaintResult {
    Painted,
    /// Current or future text does not fit; nothing was drawn.
    Overflow(Overflow),
    Failed(String),
}

/// Everything native the coordinator needs. All methods run on the UI thread and
/// none may block on Explorer or a UI Automation provider.
pub trait Platform {
    fn monotonic(&mut self) -> Duration;
    /// One fresh OS-local snapshot (zone and DST resolved by the OS).
    fn local_now(&mut self) -> DateTime<FixedOffset>;
    fn system_appearance(&mut self) -> SystemAppearance;
    /// Re-read the system UI font; true if it changed.
    fn refresh_fonts(&mut self) -> bool;
    fn shell_identity(&mut self) -> Option<ShellIdentity>;
    /// The shell changed (or vanished): retarget event hooks.
    fn shell_changed(&mut self, shell: Option<ShellIdentity>);
    /// Queue discovery for `generation`, superseding any older request. The result
    /// arrives later as [`Input::Discovery`].
    fn request_discovery(&mut self, generation: ShellGeneration);
    /// Make discovery metadata (DPI, shell epoch) available to overlay effects.
    fn note_taskbars(&mut self, snapshots: &[TaskbarSnapshot]);
    /// Cheap native window inventory only (no UI Automation). Detect taskbars
    /// created after a partial discovery, including late secondary taskbars.
    fn taskbar_inventory_changed(&mut self) -> bool {
        false
    }
    fn observe(&mut self, taskbar: &TaskbarSnapshot) -> Observation;
    fn create_overlay(&mut self, key: TaskbarKey, bounds: PhysicalRect) -> Result<(), String>;
    fn set_bounds(&mut self, key: TaskbarKey, bounds: PhysicalRect) -> Result<(), String>;
    fn destroy_overlay(&mut self, key: TaskbarKey);
    fn paint(&mut self, key: TaskbarKey, context: &PaintContext<'_>) -> PaintResult;
    fn hide(&mut self, key: TaskbarKey);
    /// Would `current` and every worst-case line fit a clock slot of `region` at
    /// `dpi`? Used by Settings before Apply. `None` when this platform cannot measure.
    fn plan_fit(
        &mut self,
        _current: &ClockText,
        _worst_case: &WorstCase,
        _region: PixelSize,
        _dpi: u32,
        _font_pt: u8,
    ) -> Option<Result<Fit, RenderError>> {
        None
    }
    /// Send the single deliberate calendar action for the clicked overlay. Must
    /// not block and must never be retried by the caller.
    fn open_calendar(&mut self, key: TaskbarKey) -> CalendarOutcome;
    /// Show the fixed menu (modal) and return the chosen command. `paused` selects
    /// the Pause/Resume label.
    fn show_menu(&mut self, anchor: Option<ScreenPoint>, paused: bool) -> Option<MenuCommand>;
    /// Ask the application to open or focus Settings (R10 owns the window).
    fn open_settings(&mut self);
    /// (Re-)register the notification-area icon. Called once at start and from the
    /// one shell-replacement path; failure is expected while Explorer starts.
    fn restore_tray(&mut self) {}
    /// Replacement was paused or resumed: update anything that shows the mode.
    fn paused_changed(&mut self, _paused: bool) {}
    /// Cheap upkeep after each observation pass: for example, re-anchor overlay
    /// z-order next to its taskbar. Must not repaint or block.
    fn maintain(&mut self) {}
    /// Replace the single outstanding monotonic one-shot clock timer.
    fn arm_timer(&mut self, delay: Duration);
    /// Replace the single outstanding shell poll timer.
    fn arm_poll(&mut self, delay: Duration);
    /// Cancel timers and discovery. Called once when the runtime finishes.
    fn stop(&mut self);
}

/// Work for the coordinator. The message loop produces these; nothing else does.
#[derive(Clone, Debug)]
pub enum Input {
    Signal(Signal),
    /// The one-shot clock timer fired.
    Timer,
    /// The shell poll timer fired.
    Poll,
    Discovery(DiscoveryReport),
    /// A click, screen-reader Invoke or menu request (R09).
    Action(UserAction),
    /// Settings committed new preferences (already validated and persisted).
    Preferences(ValidatedPreferences),
    Quit,
}

/// What the user asked for through the overlay or tray. Values only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserAction {
    /// A left click, or a screen-reader Invoke, on this taskbar's overlay.
    Activate(TaskbarKey),
    /// Show the fixed menu; `None` anchors at the cursor.
    ShowMenu(Option<ScreenPoint>),
}

/// Why a taskbar has no visible clock overlay, for Settings and Support.
#[derive(Clone, Debug, PartialEq)]
pub enum Reason {
    /// No clock-only rectangle was proven for this taskbar (the shell exposed
    /// no verifiable clock). The native clock stays visible.
    ClockPositionUnverified,
    InvalidGeometry,
    /// Text does not fit the clock slot; this overlay is hidden, others unaffected.
    Overflow(Overflow),
    RenderFailed(String),
    PlacementFailed(String),
    /// The calendar action could not be sent, so overlays were removed to keep the
    /// native clock usable. Pause then Resume (or an Explorer restart) tries again.
    ActionFailed(String),
    /// Not currently shown by design (other monitor, taskbar hidden, fullscreen,
    /// locked); the reducer's [`OverlayState`] carries the specific cause.
    NotShown,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TaskbarDiagnostic {
    pub key: TaskbarKey,
    pub primary: bool,
    pub state: OverlayState,
    pub reason: Option<Reason>,
    /// Bounds of the monitor this taskbar is on, in physical pixels.
    pub monitor_rect: PhysicalRect,
    pub dpi: u32,
    pub strategy: DiscoveryStrategy,
    /// Whether clock-only bounds were proven for this taskbar.
    pub clock_located: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostics {
    pub generation: ShellGeneration,
    /// The user paused replacement: the native clock is showing on purpose.
    pub paused: bool,
    pub taskbars: Vec<TaskbarDiagnostic>,
    pub last_discovery_failure: Option<DiscoveryFailure>,
    pub consecutive_failures: u32,
    pub discovery_disabled: bool,
    /// Windows Explorer (the shell) was running when last checked.
    pub shell_running: bool,
    /// A discovery is running or about to start.
    pub discovery_pending: bool,
    /// Discovery results processed so far; Support uses it to see a new answer.
    pub discovery_completed: u64,
}

/// The last state written to the log, so that only changes are logged.
#[derive(Default)]
struct Logged {
    taskbars: BTreeMap<u64, String>,
    failure: Option<String>,
    paused: bool,
    disabled: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Retry {
    failures: u32,
    due: Option<Duration>,
}
impl Retry {
    fn failed(&mut self, now: Duration) {
        self.failures = self.failures.saturating_add(1);
        let shift = (self.failures - 1).min(5);
        self.due = Some(now + (RECOVERY_POLL * (1 << shift)).min(RETRY_CAP));
    }
    fn reset(&mut self) {
        *self = Self::default();
    }
}

pub struct Runtime<P: Platform> {
    platform: P,
    state: Option<State>,
    tracker: AppearanceTracker,
    theme_revision: u64,
    worst_case: WorstCase,
    shell: Option<ShellIdentity>,
    snapshots: BTreeMap<u64, TaskbarSnapshot>,
    reported: HashMap<TaskbarKey, (TaskbarVisibility, bool)>,
    faults: HashMap<TaskbarKey, Reason>,
    relocation_seen: BTreeMap<u64, PhysicalRect>,
    in_flight: Option<ShellGeneration>,
    topology_dirty: bool,
    settle_due: Option<Duration>,
    transition: TransitionPoll,
    poll_due: Option<Duration>,
    refresh_not_before: Duration,
    retry: Retry,
    discovery_completed: u64,
    logged: Logged,
    last_failure: Option<DiscoveryFailure>,
    discovery_disabled: bool,
    activation: ActivationGate,
    menu: ActivationGate,
    finished: bool,
}

impl<P: Platform> Runtime<P> {
    pub fn new(mut platform: P, preferences: ValidatedPreferences) -> Self {
        let tracker = AppearanceTracker::new(platform.system_appearance());
        Self {
            worst_case: WorstCase::for_format(preferences.format()),
            platform,
            state: Some(State::new(preferences)),
            tracker,
            theme_revision: 0,
            shell: None,
            snapshots: BTreeMap::new(),
            reported: HashMap::new(),
            faults: HashMap::new(),
            relocation_seen: BTreeMap::new(),
            in_flight: None,
            topology_dirty: false,
            settle_due: None,
            transition: TransitionPoll::default(),
            poll_due: None,
            refresh_not_before: Duration::ZERO,
            retry: Retry::default(),
            discovery_completed: 0,
            logged: Logged::default(),
            last_failure: None,
            discovery_disabled: false,
            activation: ActivationGate::default(),
            menu: ActivationGate::default(),
            finished: false,
        }
    }

    /// Begin: first time snapshot (arms the clock timer), first discovery, and
    /// the recovery poll. Idempotent calls after `Quit` do nothing.
    pub fn start(&mut self) {
        if self.finished {
            return;
        }
        self.shell = self.platform.shell_identity();
        self.platform.shell_changed(self.shell);
        self.platform.restore_tray();
        let now = self.platform.local_now();
        self.dispatch(Event::Tick(now));
        self.dispatch(Event::TopologyChanged);
        self.rearm_poll();
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
    pub fn platform(&self) -> &P {
        &self.platform
    }
    pub fn platform_mut(&mut self) -> &mut P {
        &mut self.platform
    }
    pub fn state(&self) -> &State {
        self.state.as_ref().expect("state is restored after reduce")
    }

    pub fn handle(&mut self, input: Input) {
        if self.finished {
            return;
        }
        match input {
            Input::Signal(signal) => self.signal(signal),
            Input::Timer => {
                let now = self.platform.local_now();
                self.dispatch(Event::Tick(now));
            }
            Input::Poll => {
                self.poll_due = None;
                self.poll();
            }
            Input::Discovery(report) => self.discovery(report),
            Input::Action(action) => self.action(action),
            Input::Preferences(preferences) => {
                if self.state().preferences().format_source() != preferences.format_source() {
                    self.worst_case = WorstCase::for_format(preferences.format());
                }
                self.dispatch(Event::PreferencesApplied(preferences));
            }
            Input::Quit => self.quit(),
        }
        if !self.finished {
            self.rearm_poll();
        }
        self.log_transitions();
    }

    /// Write what *changed* since the last call to the bounded log: one line per
    /// taskbar outcome, discovery failure or recovery, pause and resume. Repaints
    /// and ticks change none of these, so they never reach the log.
    fn log_transitions(&mut self) {
        if !logging::enabled() {
            return;
        }
        for line in self.transition_lines() {
            logging::log(&line);
        }
    }

    /// The lines for what changed since the previous call (and remembers the new
    /// state). Pure apart from that memory, so it is tested without a log file.
    fn transition_lines(&mut self) -> Vec<String> {
        let diagnostics = self.diagnostics();
        let mut lines: Vec<String> = Vec::new();
        let mut seen: BTreeMap<u64, String> = BTreeMap::new();
        for bar in &diagnostics.taskbars {
            let summary = support::taskbar_summary(bar, diagnostics.paused);
            if self.logged.taskbars.get(&bar.key.id.0) != Some(&summary) {
                lines.push(format!(
                    "{} taskbar {}: {summary}",
                    if bar.primary { "primary" } else { "secondary" },
                    bar.key.id.0,
                ));
            }
            seen.insert(bar.key.id.0, summary);
        }
        for id in self.logged.taskbars.keys() {
            if !seen.contains_key(id) {
                lines.push(format!("taskbar {id}: gone"));
            }
        }
        self.logged.taskbars = seen;
        let failure = diagnostics.last_discovery_failure.map(|f| f.to_string());
        if failure != self.logged.failure {
            lines.push(match &failure {
                Some(why) => format!(
                    "taskbar discovery failed ({} in a row): {why}",
                    diagnostics.consecutive_failures
                ),
                None => "taskbar discovery recovered".to_owned(),
            });
            self.logged.failure = failure;
        }
        if diagnostics.paused != self.logged.paused {
            lines.push(
                if diagnostics.paused {
                    "replacement paused"
                } else {
                    "replacement resumed"
                }
                .to_owned(),
            );
            self.logged.paused = diagnostics.paused;
        }
        if diagnostics.discovery_disabled && !self.logged.disabled {
            lines.push("taskbar discovery turned off for this session".to_owned());
            self.logged.disabled = true;
        }
        lines
    }

    pub fn handle_signals(&mut self, signals: impl IntoIterator<Item = Signal>) {
        for signal in signals {
            self.handle(Input::Signal(signal));
        }
    }

    /// Taskbars whose clock slot is known and that a given display selection would
    /// use, as (is primary, slot size, DPI).
    fn fit_targets(&self, monitors: Monitors) -> Vec<(bool, PixelSize, u32)> {
        self.snapshots
            .values()
            .filter(|s| monitors == Monitors::All || s.primary)
            .filter_map(|s| Some((s.primary, PixelSize::of(s.clock_rect?), s.dpi)))
            .collect()
    }

    pub fn diagnostics(&self) -> Diagnostics {
        let state = self.state();
        let taskbars = self
            .snapshots
            .values()
            .map(|snapshot| {
                let key = snapshot.key;
                let overlay = state.overlay_state(key);
                // A render/placement fault overrides the reducer's view: the reducer
                // believes the overlay is shown, but the runtime hid it.
                let reason = match (self.faults.get(&key), overlay) {
                    (Some(fault), _) => Some(fault.clone()),
                    (None, OverlayState::Visible) => None,
                    (None, OverlayState::Unavailable) if snapshot.clock_rect.is_none() => {
                        Some(Reason::ClockPositionUnverified)
                    }
                    (None, OverlayState::Unavailable) => Some(Reason::InvalidGeometry),
                    (None, OverlayState::Hidden(_)) => Some(Reason::NotShown),
                };
                TaskbarDiagnostic {
                    key,
                    primary: snapshot.primary,
                    state: overlay,
                    reason,
                    monitor_rect: snapshot.monitor_rect,
                    dpi: snapshot.dpi,
                    strategy: snapshot.strategy,
                    clock_located: snapshot.clock_rect.is_some(),
                }
            })
            .collect();
        Diagnostics {
            generation: state.generation(),
            paused: state.is_paused(),
            taskbars,
            last_discovery_failure: self.last_failure.clone(),
            consecutive_failures: self.retry.failures,
            discovery_disabled: self.discovery_disabled,
            shell_running: self.shell.is_some(),
            discovery_pending: self.in_flight.is_some() || self.settle_due.is_some(),
            discovery_completed: self.discovery_completed,
        }
    }

    /// Settings → Support "Refresh status": look at the taskbars again now. This is
    /// the ordinary bounded discovery (a helper process with a hard deadline, never
    /// UI Automation on this thread), requested as if the display layout changed.
    /// The answer arrives later as [`Input::Discovery`] and bumps
    /// `Diagnostics::discovery_completed`; a stuck or failing helper only produces a
    /// failure report, so callers can always stop waiting.
    pub fn refresh_status(&mut self) {
        if self.finished {
            return;
        }
        logging::log("status refresh requested");
        self.topology_changed();
        self.rearm_poll();
    }

    // ---- signals -------------------------------------------------------

    fn signal(&mut self, signal: Signal) {
        match signal {
            Signal::TopologyChanged => {
                self.topology_changed();
                self.observe_all();
            }
            Signal::AppearanceMaybeChanged => self.appearance_changed(),
            Signal::TimeChanged => {
                let now = self.platform.local_now();
                self.dispatch(Event::TimeChanged(now));
            }
            Signal::Suspended => {}
            Signal::Resumed => {
                // Sleep invalidates everything the runtime inferred: wall time,
                // the monitor set, Explorer and appearance.
                let now = self.platform.local_now();
                self.dispatch(Event::TimeChanged(now));
                self.check_shell(false);
                self.appearance_changed();
                self.topology_changed();
            }
            Signal::SessionLocked => self.dispatch(Event::SessionLocked),
            Signal::SessionUnlocked => {
                self.dispatch(Event::SessionUnlocked);
                // The reducer asks for discovery; the runtime owes a fresh time.
                let now = self.platform.local_now();
                self.dispatch(Event::TimeChanged(now));
                self.check_shell(false);
            }
            Signal::TaskbarCreated => self.check_shell(true),
            Signal::Shell(ShellEvent::TaskbarChanged) => {
                self.transition.begin();
                self.observe_all();
            }
            Signal::Shell(ShellEvent::ForegroundChanged) => {
                self.observe_all();
            }
            Signal::EndSession => self.quit(),
        }
    }

    fn appearance_changed(&mut self) {
        let fonts = self.platform.refresh_fonts();
        let appearance = self.platform.system_appearance();
        if self.tracker.observe(appearance).is_some() || fonts {
            self.theme_revision = self.theme_revision.wrapping_add(1);
            self.dispatch(Event::ThemeChanged {
                revision: self.theme_revision,
            });
        }
    }

    fn topology_changed(&mut self) {
        if self.discovery_disabled {
            return;
        }
        // A real topology event overrides failure backoff: try again promptly.
        self.retry.reset();
        if self.in_flight.is_some() {
            // The reducer coalesces requests while one is pending; remember to
            // refresh afterwards, since the in-flight answer may predate this.
            self.topology_dirty = true;
        } else if self.settle_due.is_none() {
            let now = self.platform.monotonic();
            self.settle_due = Some(now + TOPOLOGY_SETTLE);
        }
    }

    /// Compare the live shell identity with the last known one. `force` is for
    /// `TaskbarCreated`, which proves the taskbar was recreated even if Explorer
    /// kept its process.
    fn check_shell(&mut self, force: bool) {
        let current = self.platform.shell_identity();
        if force || current != self.shell {
            self.shell = current;
            self.platform.shell_changed(current);
            // Explorer's notification area was rebuilt along with its taskbar: put
            // the icon back from this same path. If the shell is not ready yet the
            // registration fails quietly and the following `TaskbarCreated` retries.
            self.platform.restore_tray();
            self.snapshots.clear();
            self.reported.clear();
            self.faults.clear();
            self.relocation_seen.clear();
            self.topology_dirty = false;
            self.settle_due = None;
            self.retry.reset();
            self.dispatch(Event::ExplorerLost);
        }
    }

    // ---- timers and polling ---------------------------------------------

    fn poll(&mut self) {
        self.check_shell(false);
        let now = self.platform.monotonic();
        if self.settle_due.is_some_and(|due| due <= now) {
            self.settle_due = None;
            self.dispatch(Event::TopologyChanged);
        }
        if !self.discovery_disabled
            && self.in_flight.is_none()
            && self.retry.due.is_some_and(|due| due <= now)
        {
            self.retry.due = None;
            self.dispatch(Event::TopologyChanged);
        }
        let changed = self.observe_all();
        if self.transition.active() {
            self.transition.observed(changed);
        }
    }

    /// Keep exactly one poll timer, due at the earliest needed moment. An armed
    /// timer that is already due sooner is left alone, so frequent clock ticks
    /// cannot keep postponing the slow recovery poll.
    fn rearm_poll(&mut self) {
        let now = self.platform.monotonic();
        let mut delay = self.transition.interval();
        for due in [self.settle_due, self.retry.due].into_iter().flatten() {
            delay = delay.min(due.saturating_sub(now).max(Duration::from_millis(10)));
        }
        let desired = now + delay;
        if self.poll_due.is_none_or(|armed| desired < armed) {
            self.poll_due = Some(desired);
            self.platform.arm_poll(delay);
        }
    }

    // ---- observation ------------------------------------------------------

    /// Read each known taskbar once and feed any difference to the reducer.
    /// Returns whether anything changed (drives transition polling).
    fn observe_all(&mut self) -> bool {
        let mut changed = false;
        let inventory_changed = self.platform.taskbar_inventory_changed();
        let mut relocated: Vec<(u64, Option<PhysicalRect>)> = Vec::new();
        let snapshots: Vec<_> = self.snapshots.values().cloned().collect();
        for snapshot in snapshots {
            let key = snapshot.key;
            let observation = self.platform.observe(&snapshot);
            let placement = shell::classify(&snapshot, &observation);
            if matches!(placement, Placement::Gone | Placement::Relocated) {
                // Geometry is no longer trustworthy. Release the native clock
                // immediately, rather than covering it until discovery finishes.
                let reported = self
                    .reported
                    .entry(key)
                    .or_insert((snapshot.visibility, false));
                if reported.0 != TaskbarVisibility::Unknown {
                    reported.0 = TaskbarVisibility::Unknown;
                    self.dispatch(Event::VisibilityChanged {
                        key,
                        visibility: TaskbarVisibility::Unknown,
                    });
                    changed = true;
                }
            }
            match placement {
                // A vanished taskbar may be momentary (Explorer is rebuilding) or
                // its monitor was unplugged; a moved one is somewhere the snapshot
                // does not describe. Both mean "rediscover".
                Placement::Gone => relocated.push((key.id.0, None)),
                Placement::Relocated => {
                    if self.relocation_seen.get(&key.id.0) != observation.rect.as_ref() {
                        relocated.push((key.id.0, observation.rect));
                    }
                }
                Placement::Present(visibility) => {
                    self.relocation_seen.remove(&key.id.0);
                    let (reported_visibility, reported_fullscreen) = self
                        .reported
                        .get(&key)
                        .copied()
                        .unwrap_or((snapshot.visibility, false));
                    if visibility != reported_visibility {
                        // A parked discovery has no validated clock rectangle.
                        // Even if its bounds did not move, a reveal needs a fresh
                        // measurement; never extrapolate an offscreen candidate.
                        if visibility == TaskbarVisibility::Visible && snapshot.clock_rect.is_none()
                        {
                            relocated.push((key.id.0, observation.rect));
                        }
                        self.reported.insert(key, (visibility, reported_fullscreen));
                        self.dispatch(Event::VisibilityChanged { key, visibility });
                        changed = true;
                    }
                    if observation.fullscreen != reported_fullscreen {
                        self.reported
                            .insert(key, (visibility, observation.fullscreen));
                        self.dispatch(Event::FullscreenChanged {
                            key,
                            fullscreen: observation.fullscreen,
                        });
                        changed = true;
                    }
                }
            }
        }
        self.platform.maintain();
        // After a failed discovery the authoritative batch is empty. The live
        // inventory then differs until recovery succeeds; let failure backoff
        // own that retry instead of resetting it on every poll.
        if (inventory_changed && self.last_failure.is_none()) || !relocated.is_empty() {
            changed = true;
            // Throttle: an inconsistent shell must not turn polling into a
            // discovery loop. A skipped refresh is retried by the next poll.
            let now = self.platform.monotonic();
            if now >= self.refresh_not_before {
                self.refresh_not_before = now + RECOVERY_POLL;
                for (id, rect) in relocated {
                    if let Some(rect) = rect {
                        self.relocation_seen.insert(id, rect);
                    }
                }
                self.topology_changed();
            }
        }
        changed
    }

    // ---- discovery --------------------------------------------------------

    fn discovery(&mut self, report: DiscoveryReport) {
        if matches!(report.outcome, Err(DiscoveryFailure::TeardownFailed)) {
            // Fatal and independent of generation: the worker has stopped for good,
            // so whatever request is outstanding will never be answered. Release
            // the reducer with an authoritative empty batch (native clock shows).
            self.discovery_disabled = true;
            self.last_failure = Some(DiscoveryFailure::TeardownFailed);
            self.in_flight = None;
            let generation = self.state().generation();
            self.accept(generation, Vec::new());
            return;
        }
        // Look at the live shell first: a result for a replaced Explorer must not
        // be mistaken for current, and detecting the change advances the generation.
        self.check_shell(false);
        let state = self.state();
        if report.generation != state.generation() || self.in_flight != Some(report.generation) {
            return; // Superseded or duplicate: the reducer would reject it too.
        }
        let generation = report.generation;
        self.in_flight = None;
        let outcome = match report.outcome {
            Ok(_) if report.shell != self.shell => Err(DiscoveryFailure::Rejected(
                "shell identity changed during discovery".into(),
            )),
            Ok(batch) if batch.is_empty() && self.shell.is_some() => {
                Err(DiscoveryFailure::NoTaskbar)
            }
            other => other,
        };
        match outcome {
            Ok(snapshots) => {
                self.last_failure = None;
                self.retry.reset();
                self.accept(generation, snapshots);
                if std::mem::take(&mut self.topology_dirty) {
                    self.topology_changed();
                }
            }
            Err(failure) => {
                if failure == DiscoveryFailure::TeardownFailed {
                    self.discovery_disabled = true;
                }
                self.last_failure = Some(failure);
                // Fail closed: an authoritative empty batch removes every overlay
                // and releases the reducer's "discovery pending" state.
                self.accept(generation, Vec::new());
                if !self.discovery_disabled {
                    let now = self.platform.monotonic();
                    self.retry.failed(now);
                }
                self.topology_dirty = false;
            }
        }
    }

    fn accept(&mut self, generation: ShellGeneration, snapshots: Vec<TaskbarSnapshot>) {
        self.platform.note_taskbars(&snapshots);
        self.discovery_completed = self.discovery_completed.wrapping_add(1);
        let previous = std::mem::take(&mut self.reported);
        self.snapshots = snapshots.iter().map(|s| (s.key.id.0, s.clone())).collect();
        self.reported = snapshots
            .iter()
            .map(|s| {
                let fullscreen = previous.get(&s.key).is_some_and(|r| r.1);
                (s.key, (s.visibility, fullscreen))
            })
            .collect();
        self.faults.retain(|key, _| self.reported.contains_key(key));
        self.relocation_seen
            .retain(|id, _| self.snapshots.contains_key(id));
        self.dispatch(Event::DiscoveryResult {
            generation,
            snapshots,
        });
        // New taskbars start with fullscreen=false in the reducer; read the truth.
        self.observe_all();
    }

    // ---- reducer plumbing ---------------------------------------------------

    fn dispatch(&mut self, event: Event) {
        let state = self.state.take().expect("state is restored after reduce");
        let (state, effects) = reduce(state, event);
        let quitting = state.is_quitting();
        self.state = Some(state);
        self.apply(effects);
        if quitting && !self.finished {
            self.finish();
        }
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Discover(generation) => {
                    if self.discovery_disabled {
                        // Keep the reducer consistent: answer with an empty batch
                        // on the next turn rather than starting a helper.
                        self.in_flight = Some(generation);
                        self.disabled_discovery_reply(generation);
                    } else {
                        self.in_flight = Some(generation);
                        self.platform.request_discovery(generation);
                    }
                }
                Effect::CreateOverlay { key, bounds } => {
                    if let Err(reason) = self.platform.create_overlay(key, bounds) {
                        self.faults.insert(key, Reason::PlacementFailed(reason));
                    }
                }
                Effect::SetBounds { key, bounds } => {
                    if let Err(reason) = self.platform.set_bounds(key, bounds) {
                        self.faults.insert(key, Reason::PlacementFailed(reason));
                    }
                }
                Effect::DestroyOverlay(key) => {
                    self.platform.destroy_overlay(key);
                    self.faults.remove(&key);
                }
                Effect::Hide(key) => self.platform.hide(key),
                Effect::Paint { key, text } => self.paint(key, &text),
                Effect::ArmTimer(delay) => self.platform.arm_timer(delay),
            }
        }
    }

    fn paint(&mut self, key: TaskbarKey, text: &ClockText) {
        if matches!(
            self.faults.get(&key),
            Some(Reason::PlacementFailed(_) | Reason::ActionFailed(_))
        ) {
            // Either no window exists for this taskbar, or the overlay was removed
            // on purpose so the native clock stays reachable.
            return;
        }
        let appearance = self.tracker.current();
        let state = self.state.as_ref().expect("state is restored after reduce");
        let result = self.platform.paint(
            key,
            &PaintContext {
                text,
                preferences: state.preferences(),
                worst_case: &self.worst_case,
                appearance,
            },
        );
        match result {
            PaintResult::Painted => {
                self.faults.remove(&key);
            }
            // Only this overlay is removed from view: the native clock shows
            // through for it, and the reason is surfaced in diagnostics.
            PaintResult::Overflow(overflow) => {
                self.platform.hide(key);
                self.faults.insert(key, Reason::Overflow(overflow));
            }
            PaintResult::Failed(reason) => {
                self.platform.hide(key);
                self.faults.insert(key, Reason::RenderFailed(reason));
            }
        }
    }

    // ---- user actions -------------------------------------------------------

    fn action(&mut self, action: UserAction) {
        match action {
            UserAction::Activate(key) => self.activate(key),
            UserAction::ShowMenu(anchor) => self.show_menu(anchor),
        }
    }

    fn activate(&mut self, key: TaskbarKey) {
        // Only an overlay that is actually in front of the native clock may act. A
        // click queued before a pause, fullscreen change, fault or Explorer restart
        // is stale; the reducer already forgets keys of older generations.
        if self.state().overlay_state(key) != OverlayState::Visible
            || self.faults.contains_key(&key)
        {
            return;
        }
        let now = self.platform.monotonic();
        if !self.activation.ready(now) {
            return;
        }
        match self.platform.open_calendar(key) {
            CalendarOutcome::Sent => self.activation.record(now),
            // The user is holding a modifier: nothing was sent, nothing is wrong.
            CalendarOutcome::ModifiersHeld => {}
            CalendarOutcome::Failed(reason) => self.release_native_clock(reason),
        }
    }

    /// The calendar action failed for a reason we cannot classify (input blocked,
    /// partial insertion). Never retry: take every overlay away so the user keeps
    /// full native access, and say why in diagnostics.
    fn release_native_clock(&mut self, reason: String) {
        let keys: Vec<_> = self.snapshots.values().map(|s| s.key).collect();
        for key in keys {
            self.platform.hide(key);
            self.faults
                .insert(key, Reason::ActionFailed(reason.clone()));
        }
    }

    fn show_menu(&mut self, anchor: Option<ScreenPoint>) {
        // The tray reports mouse and keyboard selection separately and Windows may
        // send both for one gesture; a menu that just closed is not reopened.
        let now = self.platform.monotonic();
        if !self.menu.ready(now) {
            return;
        }
        let paused = self.state().is_paused();
        let command = self.platform.show_menu(anchor, paused);
        let closed = self.platform.monotonic();
        self.menu.record(closed);
        if let Some(command) = command {
            self.command(command);
        }
    }

    fn command(&mut self, command: MenuCommand) {
        match command {
            MenuCommand::Settings => self.platform.open_settings(),
            MenuCommand::TogglePause => {
                let paused = !self.state().is_paused();
                self.set_paused(paused);
            }
            MenuCommand::Exit => self.quit(),
        }
    }

    fn set_paused(&mut self, paused: bool) {
        if self.state().is_paused() == paused {
            return;
        }
        // Pause then Resume is the user's retry for an overlay removed after a
        // failed calendar action.
        self.faults
            .retain(|_, reason| !matches!(reason, Reason::ActionFailed(_)));
        self.dispatch(Event::SetPaused(paused));
        self.platform.paused_changed(paused);
    }

    fn disabled_discovery_reply(&mut self, generation: ShellGeneration) {
        self.in_flight = None;
        self.accept(generation, Vec::new());
    }

    fn quit(&mut self) {
        self.dispatch(Event::Quit);
        self.finish();
    }

    fn finish(&mut self) {
        if !self.finished {
            self.finished = true;
            self.in_flight = None;
            self.snapshots.clear();
            self.platform.stop();
        }
    }
}

/// Settings asks the runtime, which owns the measured clock slots and the renderer,
/// whether a draft would fit before it is applied.
impl<P: Platform> LayoutCheck for Runtime<P> {
    fn check(
        &mut self,
        format: &CompiledFormat,
        font_pt: u8,
        monitors: Monitors,
        now: DateTime<FixedOffset>,
    ) -> Vec<FitProblem> {
        let targets = self.fit_targets(monitors);
        let Ok(current) = format.render(now) else {
            return Vec::new();
        };
        if targets.is_empty() {
            return Vec::new();
        }
        let worst_case = WorstCase::for_format(format);
        targets
            .into_iter()
            .filter_map(|(primary, region, dpi)| {
                match self
                    .platform
                    .plan_fit(&current, &worst_case, region, dpi, font_pt)
                {
                    Some(Ok(Fit::Overflow(overflow))) => Some(FitProblem { primary, overflow }),
                    // Fits, cannot be measured here, or measuring failed: the paint
                    // path still decides and reports through diagnostics.
                    _ => None,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
