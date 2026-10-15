//! Clock actions, the fixed app menu and click pairing (R09).
//!
//! Everything here is portable value logic that tests on any host: which
//! commands exist, when a mouse press becomes one activation, how repeated
//! activations coalesce, and which keystrokes the deliberate calendar action
//! sends (and releases after a partial insertion). Native input, the popup menu
//! and the window procedures live in `native`.
//!
//! ## Action behavior
//!
//! | Source | Result |
//! | --- | --- |
//! | Left press **and** release inside the overlay | One native calendar action (Win+N, see below) |
//! | Press released outside / capture lost | Nothing |
//! | Right click on the overlay | Fixed menu at the pointer |
//! | Tray icon: context menu, select or keyboard activation | The same fixed menu |
//! | Screen-reader Invoke on the overlay's UIA element | Same as a left click |
//! | Menu *Settings* / *Pause* / *Resume* / *Exit* | Open or focus Settings / toggle replacement / quit |
//!
//! There is no arbitrary-command, double-click, middle-click or shortcut
//! customization, and the overlay never relays a mouse click.
//!
//! ## Calendar strategy
//!
//! The R03 hosted captures showed UI Automation `Invoke` on the native clock
//! returning success without opening the flyout, while one deliberate Win+N
//! opened it. The production action is therefore a single four-event `SendInput`
//! batch on the UI thread. It never calls COM/UIA from the message loop, never
//! retries, refuses to start while the user physically holds a modifier or `N`
//! (the combination would not mean Win+N), and releases any key whose injected
//! key-down was accepted when the batch is only partly inserted.
use std::{collections::VecDeque, time::Duration};

/// A second activation inside this window is a double click or a stuck-key
/// repeat, not a deliberate second request. Win+N toggles the flyout, so without
/// this a double click would open and immediately close the calendar.
pub const ACTIVATION_COOLDOWN: Duration = Duration::from_millis(400);

/// Upper bound on pending interactions. A flood keeps the newest.
const QUEUE_CAP: usize = 16;

/// A physical screen position, for placing the menu.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScreenPoint {
    pub x: i32,
    pub y: i32,
}

/// The fixed menu. Exactly these commands exist; nothing is configurable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuCommand {
    Settings,
    TogglePause,
    Exit,
}
impl MenuCommand {
    pub const ALL: [Self; 3] = [Self::Settings, Self::TogglePause, Self::Exit];
    /// Stable, non-zero menu item identifier (zero means "no selection").
    pub const fn id(self) -> u32 {
        match self {
            Self::Settings => 1,
            Self::TogglePause => 2,
            Self::Exit => 3,
        }
    }
    pub fn from_id(id: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|command| command.id() == id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MenuItem {
    pub command: MenuCommand,
    pub label: &'static str,
    /// Drawn bold and chosen by Enter on the menu's first item.
    pub default: bool,
}
/// The menu for the current state. The pause item names the action it performs.
pub fn menu_items(paused: bool) -> [MenuItem; 3] {
    [
        MenuItem {
            command: MenuCommand::Settings,
            label: "Settings",
            default: true,
        },
        MenuItem {
            command: MenuCommand::TogglePause,
            label: if paused {
                "Resume clock replacement"
            } else {
                "Pause clock replacement"
            },
            default: false,
        },
        MenuItem {
            command: MenuCommand::Exit,
            label: "Exit",
            default: false,
        },
    ]
}
/// Tray tooltip. It states the mode, because a paused replacement is otherwise
/// indistinguishable from the original clock.
pub fn tray_tooltip(paused: bool) -> &'static str {
    if paused {
        "ElevenClock-rs (paused)"
    } else {
        "ElevenClock-rs"
    }
}

/// Pairs a button press with its release so only a click that starts and ends on
/// the same overlay activates it. Windows delivers the release to the capturing
/// window even when the pointer left, so `inside` is decided by the caller.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PressTracker {
    pressed: Option<u64>,
}
impl PressTracker {
    pub const fn new() -> Self {
        Self { pressed: None }
    }
    pub fn down(&mut self, window: u64) {
        self.pressed = Some(window);
    }
    /// True exactly once per completed click.
    pub fn up(&mut self, window: u64, inside: bool) -> bool {
        self.pressed.take() == Some(window) && inside
    }
    /// Capture was lost (focus change, another press, window destroyed).
    pub fn cancel(&mut self) {
        self.pressed = None;
    }
    pub fn is_pressed(&self) -> bool {
        self.pressed.is_some()
    }
}

/// Rate limit for the calendar action.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ActivationGate {
    last: Option<Duration>,
}
impl ActivationGate {
    pub fn ready(&self, now: Duration) -> bool {
        self.last
            .is_none_or(|last| now.saturating_sub(last) >= ACTIVATION_COOLDOWN || now < last)
    }
    /// Record a calendar action that was actually sent.
    pub fn record(&mut self, now: Duration) {
        self.last = Some(now);
    }
}

/// What a native callback asks the loop to do. Values only; no handles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Interaction {
    /// Open the native calendar for the overlay window with this id.
    Activate { window: u64 },
    /// Show the fixed menu; `None` anchors to the cursor.
    Menu { anchor: Option<ScreenPoint> },
}

/// Coalescing bounded queue between window procedures (and the UIA provider
/// thread) and the message loop: one pending activation per overlay, one pending
/// menu. Two clicks before the loop runs are therefore one action.
#[derive(Debug, Default)]
pub struct InteractionQueue {
    items: VecDeque<Interaction>,
}
impl InteractionQueue {
    pub fn push(&mut self, interaction: Interaction) {
        let duplicate = self
            .items
            .iter()
            .any(|queued| match (queued, &interaction) {
                (Interaction::Activate { window: a }, Interaction::Activate { window: b }) => {
                    a == b
                }
                (Interaction::Menu { .. }, Interaction::Menu { .. }) => true,
                _ => false,
            });
        if duplicate {
            return;
        }
        if self.items.len() == QUEUE_CAP {
            self.items.pop_front();
        }
        self.items.push_back(interaction);
    }
    pub fn take(&mut self) -> Vec<Interaction> {
        self.items.drain(..).collect()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Keys the calendar action presses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Win,
    N,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    pub key: Key,
    pub up: bool,
}
const fn press(key: Key) -> KeyEvent {
    KeyEvent { key, up: false }
}
const fn release(key: Key) -> KeyEvent {
    KeyEvent { key, up: true }
}
/// The complete batch, inserted atomically by one `SendInput` call.
pub const WIN_N: [KeyEvent; 4] = [
    press(Key::Win),
    press(Key::N),
    release(Key::N),
    release(Key::Win),
];
/// Key-ups that undo a batch of which only the first `inserted` events were
/// accepted, so no injected key stays down. Order: `N` before Win.
pub fn cleanup_after(inserted: usize) -> Vec<KeyEvent> {
    let mut out = Vec::new();
    // After 2 events N is down; after 3 only Win is still down.
    if inserted == 2 {
        out.push(release(Key::N));
    }
    if (1..WIN_N.len()).contains(&inserted) {
        out.push(release(Key::Win));
    }
    out
}

/// Keys the user holds at the moment of the action.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HeldKeys {
    pub win: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub n: bool,
}
impl HeldKeys {
    /// Injecting Win+N on top of a held modifier would form a different shortcut
    /// (for example Ctrl+Win+N), and releasing the user's own keys is not ours to do.
    pub fn blocks_calendar(&self) -> bool {
        self.win || self.ctrl || self.shift || self.alt || self.n
    }
}

/// Result of one calendar request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CalendarOutcome {
    /// The single action was sent.
    Sent,
    /// Nothing was sent and nothing is wrong: the user is holding a modifier.
    ModifiersHeld,
    /// The action could not be sent. The runtime removes the overlay so the
    /// native clock stays reachable, and records this reason.
    Failed(String),
}

/// The accessible name of the displayed clock: its lines in reading order. UIA
/// carries no line breaks well, and a very long format must not become an
/// unbounded property.
pub fn accessible_name(lines: &[String]) -> String {
    const LIMIT: usize = 256;
    let joined = lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    joined.chars().take(LIMIT).collect()
}

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub(crate) use native::{
    WM_INTERACTION, clear_host, post_interaction, send_win_n, set_host, show_menu,
    take_interactions,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_is_exactly_settings_pause_exit_with_state_aware_label() {
        let running = menu_items(false);
        let paused = menu_items(true);
        assert_eq!(
            running.map(|item| item.command),
            [
                MenuCommand::Settings,
                MenuCommand::TogglePause,
                MenuCommand::Exit
            ]
        );
        assert_eq!(running[1].label, "Pause clock replacement");
        assert_eq!(paused[1].label, "Resume clock replacement");
        assert!(running[0].default && !running[1].default && !running[2].default);
        assert_eq!(running[0].label, paused[0].label);
        assert_eq!(running[2].label, paused[2].label);
    }

    #[test]
    fn menu_ids_are_nonzero_unique_and_round_trip() {
        let ids: Vec<_> = MenuCommand::ALL.iter().map(|c| c.id()).collect();
        assert!(ids.iter().all(|id| *id != 0));
        let mut sorted = ids.clone();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
        for command in MenuCommand::ALL {
            assert_eq!(MenuCommand::from_id(command.id()), Some(command));
        }
        assert_eq!(MenuCommand::from_id(0), None);
        assert_eq!(MenuCommand::from_id(4), None);
    }

    #[test]
    fn a_click_activates_once_only_when_it_ends_on_the_window_it_started_on() {
        let mut press = PressTracker::default();
        press.down(7);
        assert!(press.is_pressed());
        assert!(press.up(7, true));
        assert!(!press.up(7, true), "a release without a press does nothing");

        press.down(7);
        assert!(!press.up(7, false), "released outside the slot");
        assert!(!press.is_pressed());

        press.down(7);
        press.cancel();
        assert!(!press.up(7, true), "capture lost");

        press.down(7);
        assert!(!press.up(8, true), "another window never completes it");
        assert!(!press.is_pressed());
    }

    #[test]
    fn rapid_activations_are_one_but_a_later_deliberate_one_works() {
        let mut gate = ActivationGate::default();
        let t = Duration::from_secs;
        assert!(gate.ready(t(10)));
        gate.record(t(10));
        assert!(!gate.ready(t(10)));
        assert!(!gate.ready(t(10) + ACTIVATION_COOLDOWN - Duration::from_millis(1)));
        assert!(gate.ready(t(10) + ACTIVATION_COOLDOWN));
        // A monotonic source that moved backwards must not lock the action out.
        assert!(gate.ready(t(5)));
        // A deferred or failed attempt is not recorded and does not consume the gate.
        let fresh = ActivationGate::default();
        assert!(fresh.ready(Duration::ZERO));
    }

    #[test]
    fn queue_coalesces_per_overlay_and_menu_and_stays_bounded() {
        let mut queue = InteractionQueue::default();
        assert!(queue.is_empty());
        queue.push(Interaction::Activate { window: 1 });
        queue.push(Interaction::Activate { window: 1 });
        queue.push(Interaction::Activate { window: 2 });
        queue.push(Interaction::Menu { anchor: None });
        queue.push(Interaction::Menu {
            anchor: Some(ScreenPoint { x: 1, y: 2 }),
        });
        assert_eq!(
            queue.take(),
            vec![
                Interaction::Activate { window: 1 },
                Interaction::Activate { window: 2 },
                Interaction::Menu { anchor: None },
            ]
        );
        assert!(queue.is_empty());
        for window in 0..100 {
            queue.push(Interaction::Activate { window });
        }
        let taken = queue.take();
        assert_eq!(taken.len(), QUEUE_CAP);
        assert_eq!(taken.last(), Some(&Interaction::Activate { window: 99 }));
    }

    #[test]
    fn calendar_batch_is_one_balanced_win_n_chord() {
        assert_eq!(
            WIN_N,
            [
                KeyEvent {
                    key: Key::Win,
                    up: false
                },
                KeyEvent {
                    key: Key::N,
                    up: false
                },
                KeyEvent {
                    key: Key::N,
                    up: true
                },
                KeyEvent {
                    key: Key::Win,
                    up: true
                },
            ]
        );
        for key in [Key::Win, Key::N] {
            let downs = WIN_N.iter().filter(|e| e.key == key && !e.up).count();
            let ups = WIN_N.iter().filter(|e| e.key == key && e.up).count();
            assert_eq!((downs, ups), (1, 1));
        }
    }

    #[test]
    fn partial_insertion_releases_exactly_the_keys_left_down() {
        let up = |key| KeyEvent { key, up: true };
        assert_eq!(cleanup_after(0), vec![]);
        assert_eq!(cleanup_after(1), vec![up(Key::Win)]);
        assert_eq!(cleanup_after(2), vec![up(Key::N), up(Key::Win)]);
        assert_eq!(cleanup_after(3), vec![up(Key::Win)]);
        assert_eq!(cleanup_after(4), vec![], "complete batch needs no cleanup");
    }

    #[test]
    fn replaying_any_prefix_with_cleanup_never_leaves_a_key_down() {
        for inserted in 0..=4 {
            let mut win = false;
            let mut n = false;
            for event in WIN_N.iter().take(inserted).chain(&cleanup_after(inserted)) {
                match event.key {
                    Key::Win => win = !event.up,
                    Key::N => n = !event.up,
                }
            }
            assert!(!win && !n, "prefix {inserted} left a key down");
        }
    }

    #[test]
    fn any_held_modifier_or_n_blocks_the_calendar_action() {
        assert!(!HeldKeys::default().blocks_calendar());
        for held in [
            HeldKeys {
                win: true,
                ..Default::default()
            },
            HeldKeys {
                ctrl: true,
                ..Default::default()
            },
            HeldKeys {
                shift: true,
                ..Default::default()
            },
            HeldKeys {
                alt: true,
                ..Default::default()
            },
            HeldKeys {
                n: true,
                ..Default::default()
            },
        ] {
            assert!(held.blocks_calendar(), "{held:?}");
        }
    }

    #[test]
    fn accessible_name_is_the_displayed_text_in_reading_order_and_bounded() {
        let lines = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
        assert_eq!(
            accessible_name(&lines(&["12:34", "14/10/2026"])),
            "12:34, 14/10/2026"
        );
        assert_eq!(accessible_name(&lines(&["  12:34  ", ""])), "12:34");
        assert_eq!(accessible_name(&[]), "");
        let long = lines(&[&"x".repeat(1000)]);
        assert_eq!(accessible_name(&long).chars().count(), 256);
    }

    #[test]
    fn tooltip_states_the_mode() {
        assert_eq!(tray_tooltip(false), "ElevenClock-rs");
        assert!(tray_tooltip(true).contains("paused"));
    }
}
