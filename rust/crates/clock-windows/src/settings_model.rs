//! Settings drafts and the Apply / Cancel / Reset flow (R10).
//!
//! The native window (`settings`) only reads and writes controls; every rule lives
//! here so it is covered by deterministic tests on any host:
//!
//! * the window edits a [`Draft`], never the active preferences;
//! * [`SettingsController::evaluate`] turns the draft into field messages (parser
//!   errors, font bounds, fit on the selected taskbars) and a live preview;
//! * [`SettingsController::apply`] persists first and only then hands back the
//!   validated value for the runtime, so a failed save changes nothing that is
//!   live and leaves the draft editable;
//! * cancel and close discard the draft, and reset only stages defaults.
//!
//! Fit is checked through [`LayoutCheck`] (implemented by the runtime with the
//! measured clock slots and DirectWrite), so the tests supply a fake layout.
use crate::{
    preferences_store::{PreferenceSession, PreferencesStore, StoreError},
    render::Overflow,
};
use chrono::{DateTime, Datelike, FixedOffset, TimeZone};
use clock_core::{
    CompiledFormat, FormatError, FormatErrorKind, Monitors, Preferences, PreferencesError, Theme,
    ValidatedPreferences, format::PRESETS,
};
use std::ops::Range;

pub const FONT_MIN: u8 = 6;
pub const FONT_MAX: u8 = 24;
const FONT_MESSAGE: &str = "Font size must be between 6 and 24 pt.";

/// What the tokens mean, as shown beside the format editor. Every directive the
/// core accepts is listed (a test keeps the two in step).
pub const TOKEN_HELP: &str = "\
Time: %H hour (00 to 23)   %I hour (01 to 12)   %M minute   %S second   %p AM or PM\n\
Date: %d day   %m month   %b short month   %B month   %a short weekday   %A weekday\n\
Calendar year: %Y (2026)   %y (26)\n\
ISO week-year: %G (2026)   %g (26). This is the year the ISO week belongs to, so it \
differs from the calendar year around 1 January.\n\
ISO week and weekday: %V week (01 to 53)   %u weekday (Monday = 1 to Sunday = 7)\n\
%% shows a percent sign. Press Enter for a second line (two lines at most). Other \
text is shown as typed.";

/// What the user is editing. Text fields stay text so that half-typed input can be
/// reported next to the control instead of being rejected by the control.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Draft {
    /// Always LF-separated; the edit control's CRLF is normalised on the way in.
    pub format: String,
    pub font_size: String,
    pub monitors: Monitors,
    pub theme: Theme,
}
impl Draft {
    pub fn from_preferences(preferences: &Preferences) -> Self {
        Self {
            format: preferences.format.replace("\r\n", "\n"),
            font_size: preferences.font_size_pt.to_string(),
            monitors: preferences.monitors,
            theme: preferences.theme,
        }
    }
    fn font(&self) -> Result<u8, String> {
        self.font_size
            .trim()
            .parse::<i64>()
            .ok()
            .and_then(|size| u8::try_from(size).ok())
            .filter(|size| (FONT_MIN..=FONT_MAX).contains(size))
            .ok_or_else(|| FONT_MESSAGE.to_owned())
    }
    fn preferences(&self) -> Option<Preferences> {
        Some(Preferences {
            format: self.format.clone(),
            monitors: self.monitors,
            font_size_pt: self.font().ok()?,
            theme: self.theme,
        })
    }
    /// Same effective values; "09" and "9" are the same font size.
    fn same_as(&self, other: &Draft) -> bool {
        self.format == other.format
            && self.monitors == other.monitors
            && self.theme == other.theme
            && match (self.font(), other.font()) {
                (Ok(a), Ok(b)) => a == b,
                _ => self.font_size == other.font_size,
            }
    }
}

/// The text does not fit a taskbar's clock slot, measured at that taskbar's DPI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FitProblem {
    pub primary: bool,
    pub overflow: Overflow,
}
/// Measures a candidate against the clock slots of the displays that would show
/// it. An empty answer means it fits (or there is nothing to measure yet).
pub trait LayoutCheck {
    fn check(
        &mut self,
        format: &CompiledFormat,
        font_pt: u8,
        monitors: Monitors,
        now: DateTime<FixedOffset>,
    ) -> Vec<FitProblem>;
}

/// A format error in words, and where it is in the edit control's text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatProblem {
    pub message: String,
    /// UTF-16 selection in the CRLF text of the edit control, when known.
    pub selection: Option<Range<usize>>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Field {
    Format,
    FontSize,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Problems {
    pub format: Option<FormatProblem>,
    pub font: Option<String>,
    /// Set only when format and font are valid but the text would not fit.
    pub fit: Option<String>,
}
impl Problems {
    pub fn is_empty(&self) -> bool {
        self.format.is_none() && self.font.is_none() && self.fit.is_none()
    }
    /// Where Apply should send the keyboard focus.
    pub fn first(&self) -> Option<Field> {
        if self.format.is_some() {
            Some(Field::Format)
        } else if self.font.is_some() {
            Some(Field::FontSize)
        } else if self.fit.is_some() {
            Some(Field::Format)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evaluation {
    pub problems: Problems,
    /// One line per sample, LF separated; a hint instead when the format is invalid.
    pub preview: String,
    pub dirty: bool,
    /// Index into `PRESETS`, or `None` for a custom format.
    pub preset: Option<usize>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ApplyOutcome {
    /// Saved, then ready for the runtime.
    Applied(ValidatedPreferences),
    /// Nothing was saved; the messages are in `evaluate`.
    Invalid,
    /// Saved data is unreadable: ask before replacing it, then apply with recovery.
    RecoveryRequired,
    /// The save failed (or is disabled). Live settings and the draft are unchanged.
    NotSaved(String),
}

pub struct SettingsController<S> {
    session: PreferenceSession<S>,
    draft: Draft,
}
impl<S: PreferencesStore> SettingsController<S> {
    pub fn new(session: PreferenceSession<S>) -> Self {
        let draft = Draft::from_preferences(&session.active().draft());
        Self { session, draft }
    }
    pub fn draft(&self) -> &Draft {
        &self.draft
    }
    pub fn active(&self) -> &ValidatedPreferences {
        self.session.active()
    }
    /// A new window starts from what is live, whatever an earlier one left behind.
    pub fn open(&mut self) {
        self.discard();
    }
    /// Cancel and close both land here: unsaved edits are dropped.
    pub fn discard(&mut self) {
        self.draft = Draft::from_preferences(&self.session.active().draft());
    }
    pub fn set_format(&mut self, text: &str) {
        self.draft.format = text.replace("\r\n", "\n");
    }
    pub fn set_font_size(&mut self, text: &str) {
        self.draft.font_size = text.to_owned();
    }
    pub fn set_monitors(&mut self, monitors: Monitors) {
        self.draft.monitors = monitors;
    }
    pub fn set_theme(&mut self, theme: Theme) {
        self.draft.theme = theme;
    }
    /// Stage a preset's format. Returns false for an unknown index.
    pub fn select_preset(&mut self, index: usize) -> bool {
        PRESETS.get(index).is_some_and(|preset| {
            self.draft.format = preset.format.to_owned();
            true
        })
    }
    /// Stage the defaults. Nothing is saved or applied until Apply.
    pub fn reset_to_defaults(&mut self) {
        self.draft = Draft::from_preferences(&self.session.reset_draft());
    }
    pub fn is_dirty(&self) -> bool {
        !self
            .draft
            .same_as(&Draft::from_preferences(&self.session.active().draft()))
    }
    /// Why saved data could not be used, in words for the user.
    pub fn notice(&self) -> Option<String> {
        self.session.load_error().map(ToString::to_string)
    }
    /// A newer app version wrote the data: this version must not overwrite it.
    pub fn saving_blocked(&self) -> bool {
        matches!(
            self.session.load_error(),
            Some(StoreError::UnsupportedVersion(_))
        )
    }
    pub fn first_run(&self) -> bool {
        !self.session.first_run_completed()
    }
    /// Best effort: a failure only means the welcome text shows once more.
    pub fn finish_first_run(&mut self) {
        if self.first_run() {
            let _ = self.session.complete_first_run();
        }
    }

    pub fn evaluate(&self, layout: &mut dyn LayoutCheck, now: DateTime<FixedOffset>) -> Evaluation {
        let mut problems = Problems::default();
        let font = self.draft.font();
        if let Err(message) = &font {
            problems.font = Some(message.clone());
        }
        let compiled = CompiledFormat::parse(&self.draft.format);
        let preview = match &compiled {
            Ok(format) => preview_text(format, now),
            Err(error) => {
                problems.format = Some(format_problem(&self.draft.format, error));
                "Fix the format to see a preview.".to_owned()
            }
        };
        if let (Ok(format), Ok(font_pt)) = (&compiled, &font) {
            let found = layout.check(format, *font_pt, self.draft.monitors, now);
            if !found.is_empty() {
                problems.fit = Some(fit_message(&found));
            }
        }
        Evaluation {
            problems,
            preview,
            dirty: self.is_dirty(),
            preset: PRESETS
                .iter()
                .position(|preset| preset.format == self.draft.format),
        }
    }

    /// Validate, check fit, persist, and only then return the value to apply live.
    /// `recover` is the user's confirmation to replace damaged saved data.
    pub fn apply(
        &mut self,
        layout: &mut dyn LayoutCheck,
        now: DateTime<FixedOffset>,
        recover: bool,
    ) -> ApplyOutcome {
        if !self.evaluate(layout, now).problems.is_empty() {
            return ApplyOutcome::Invalid;
        }
        let Some(preferences) = self.draft.preferences() else {
            return ApplyOutcome::Invalid;
        };
        let saved = if recover {
            self.session.recover(preferences)
        } else {
            self.session.apply(preferences)
        };
        match saved {
            Ok(()) => {
                self.discard();
                ApplyOutcome::Applied(self.session.active().clone())
            }
            Err(StoreError::RecoveryRequired) => ApplyOutcome::RecoveryRequired,
            // Cannot happen after the checks above, but never panic over it.
            Err(StoreError::InvalidPreferences(PreferencesError::FontSizeOutOfRange(_))) => {
                ApplyOutcome::NotSaved(FONT_MESSAGE.to_owned())
            }
            Err(error) => ApplyOutcome::NotSaved(error.to_string()),
        }
    }
}

fn sample(year: i32, month: u32, day: u32, offset_hours: i32) -> Option<DateTime<FixedOffset>> {
    FixedOffset::east_opt(offset_hours * 3600)?
        .with_ymd_and_hms(year, month, day, 12, 34, 56)
        .single()
}

/// The documented examples, with the ISO week-year each one belongs to so the
/// difference from the calendar year is visible rather than implied.
fn examples() -> Vec<DateTime<FixedOffset>> {
    [
        (2026, 10, 13, 2),
        (2025, 12, 29, 1),
        (2027, 1, 1, 1),
        (2027, 1, 4, 1),
    ]
    .into_iter()
    .filter_map(|(year, month, day, hours)| sample(year, month, day, hours))
    .collect()
}

fn describe(when: DateTime<FixedOffset>) -> String {
    let week = when.iso_week();
    format!(
        "{} (ISO week-year {}, week {})",
        when.format("%a %-d %b %Y"),
        week.year(),
        week.week()
    )
}

fn lines_text(lines: &[String]) -> String {
    lines.join(" \u{21b5} ")
}

fn preview_text(format: &CompiledFormat, now: DateTime<FixedOffset>) -> String {
    std::iter::once(("Now".to_owned(), now))
        .chain(examples().into_iter().map(|when| (describe(when), when)))
        .map(|(label, when)| match format.render(when) {
            Ok(text) => format!("{label}: {}", lines_text(&text.lines)),
            Err(error) => format!("{label}: cannot be shown ({})", render_reason(error.kind)),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_reason(kind: FormatErrorKind) -> &'static str {
    match kind {
        FormatErrorKind::OutputTooLong => "the result is longer than 512 characters",
        _ => "the date is out of range",
    }
}

fn format_problem(text: &str, error: &FormatError) -> FormatProblem {
    let chars: Vec<char> = text.chars().collect();
    let snippet: String = chars
        .get(error.span.clone())
        .map(|part| part.iter().collect())
        .unwrap_or_default();
    let message = match error.kind {
        FormatErrorKind::InputTooLong => "Use at most 256 characters.".to_owned(),
        FormatErrorKind::EmptyLine => {
            if chars.is_empty() {
                "Enter a format, or choose a preset.".to_owned()
            } else {
                "Each line needs some content. Remove the empty line or add a token.".to_owned()
            }
        }
        FormatErrorKind::TooManyLines => "Use at most two lines.".to_owned(),
        FormatErrorKind::ControlCharacter => {
            "Tabs and other control characters cannot be used.".to_owned()
        }
        FormatErrorKind::IncompleteDirective => {
            "A % must be followed by a token. Use %% to show a percent sign.".to_owned()
        }
        FormatErrorKind::UnsupportedDirective => format!(
            "\u{201c}{snippet}\u{201d} is not a supported token. The token list below shows what you can use."
        ),
        FormatErrorKind::OutputTooLong => {
            "The result would be longer than 512 characters.".to_owned()
        }
        FormatErrorKind::SnapshotOutOfRange => "That date cannot be shown.".to_owned(),
    };
    // Whole-input errors have no useful place to select.
    let selection = matches!(
        error.kind,
        FormatErrorKind::EmptyLine
            | FormatErrorKind::TooManyLines
            | FormatErrorKind::ControlCharacter
            | FormatErrorKind::IncompleteDirective
            | FormatErrorKind::UnsupportedDirective
            | FormatErrorKind::InputTooLong
    )
    .then(|| edit_selection(text, error.span.clone()));
    FormatProblem { message, selection }
}

/// Map a half-open span of Unicode scalars in LF text to UTF-16 units in the edit
/// control's CRLF text, where every line break counts as two units.
pub fn edit_selection(text: &str, span: Range<usize>) -> Range<usize> {
    let (mut start, mut end) = (None, None);
    let mut units = 0;
    let mut count = 0;
    for (index, ch) in text.chars().enumerate() {
        if index == span.start {
            start = Some(units);
        }
        if index == span.end {
            end = Some(units);
        }
        units += if ch == '\n' { 2 } else { ch.len_utf16() };
        count = index + 1;
    }
    if span.start >= count {
        start = Some(units);
    }
    if span.end >= count {
        end = Some(units);
    }
    let start = start.unwrap_or(units);
    start..end.unwrap_or(units).max(start)
}

fn fit_message(problems: &[FitProblem]) -> String {
    let places: Vec<String> = problems
        .iter()
        .map(|problem| {
            let o = problem.overflow;
            let which = if problem.primary {
                "the primary taskbar"
            } else {
                "a secondary taskbar"
            };
            format!(
                "{which} (needs {} \u{d7} {} px, {} \u{d7} {} px available)",
                o.required.width, o.required.height, o.available.width, o.available.height
            )
        })
        .collect();
    format!(
        "This does not fit the clock space on {}. Use a shorter format or a smaller font size.",
        places.join(" and ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        preferences_store::{MemoryPreferencesStore, StoredPreferences},
        render::PixelSize,
    };
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-10-13T09:15:00+02:00").unwrap()
    }

    #[derive(Default)]
    struct FakeLayout {
        problems: Vec<FitProblem>,
        calls: Vec<(String, u8, Monitors)>,
    }
    impl LayoutCheck for FakeLayout {
        fn check(
            &mut self,
            format: &CompiledFormat,
            font_pt: u8,
            monitors: Monitors,
            _: DateTime<FixedOffset>,
        ) -> Vec<FitProblem> {
            let text = format.render(now()).unwrap().lines.join("\n");
            self.calls.push((text, font_pt, monitors));
            self.problems.clone()
        }
    }
    fn too_wide() -> FitProblem {
        FitProblem {
            primary: true,
            overflow: Overflow {
                required: PixelSize {
                    width: 140,
                    height: 32,
                },
                available: PixelSize {
                    width: 96,
                    height: 40,
                },
            },
        }
    }

    /// Records every operation so ordering and "nothing was written" are checkable.
    #[derive(Clone, Default)]
    struct Log(Rc<RefCell<Vec<String>>>);
    impl Log {
        fn entries(&self) -> Vec<String> {
            self.0.borrow().clone()
        }
    }
    struct FakeStore {
        inner: MemoryPreferencesStore,
        log: Log,
        fail_save: Rc<Cell<bool>>,
        damaged: bool,
    }
    impl FakeStore {
        fn new(log: &Log) -> Self {
            Self {
                inner: MemoryPreferencesStore::default(),
                log: log.clone(),
                fail_save: Rc::default(),
                damaged: false,
            }
        }
    }
    impl PreferencesStore for FakeStore {
        fn load(&self) -> Result<StoredPreferences, StoreError> {
            if self.damaged {
                Err(StoreError::DamagedData)
            } else {
                self.inner.load()
            }
        }
        fn save(&mut self, value: &StoredPreferences) -> Result<(), StoreError> {
            self.log.0.borrow_mut().push("save".into());
            if self.fail_save.get() {
                return Err(StoreError::Io(std::io::Error::other("disk full")));
            }
            self.damaged = false;
            self.inner.save(value)
        }
        fn preserve_for_recovery(&mut self) -> Result<(), StoreError> {
            self.log.0.borrow_mut().push("preserve".into());
            Ok(())
        }
    }
    fn controller(store: FakeStore) -> SettingsController<FakeStore> {
        SettingsController::new(PreferenceSession::open(store))
    }
    fn fresh() -> (SettingsController<FakeStore>, Log) {
        let log = Log::default();
        (controller(FakeStore::new(&log)), log)
    }

    #[test]
    fn starts_from_the_live_values_and_is_clean() {
        let (c, _) = fresh();
        assert_eq!(c.draft().format, "%H:%M\n%g%V.%u");
        assert_eq!(c.draft().font_size, "9");
        assert!(!c.is_dirty());
        assert!(c.first_run());
        assert!(c.notice().is_none());
    }

    #[test]
    fn preset_and_format_edits_preview_the_fixture_and_year_boundaries() {
        let (mut c, _) = fresh();
        let mut layout = FakeLayout::default();
        assert!(c.select_preset(0));
        let iso = c.evaluate(&mut layout, now());
        assert_eq!(iso.preset, Some(0));
        assert!(iso.problems.is_empty());
        let rows: Vec<&str> = iso.preview.lines().collect();
        assert_eq!(rows[0], "Now: 2642.2");
        assert_eq!(
            rows[1],
            "Tue 13 Oct 2026 (ISO week-year 2026, week 42): 2642.2"
        );
        assert_eq!(
            rows[2],
            "Mon 29 Dec 2025 (ISO week-year 2026, week 1): 2601.1"
        );
        assert_eq!(
            rows[3],
            "Fri 1 Jan 2027 (ISO week-year 2026, week 53): 2653.5"
        );
        assert_eq!(
            rows[4],
            "Mon 4 Jan 2027 (ISO week-year 2027, week 1): 2701.1"
        );

        // The calendar-year preset differs exactly where the documentation says.
        assert!(c.select_preset(1));
        let calendar = c.evaluate(&mut layout, now());
        assert_eq!(calendar.preset, Some(1));
        let rows: Vec<&str> = calendar.preview.lines().collect();
        assert!(rows[1].ends_with(": 2642.2"));
        assert!(rows[2].ends_with(": 2501.1"), "{}", rows[2]);
        assert!(rows[3].ends_with(": 2753.5"), "{}", rows[3]);
        assert!(rows[4].ends_with(": 2701.1"));

        // Typing the same text is the same preset; anything else is custom.
        c.set_format("%y%V.%u");
        assert_eq!(c.evaluate(&mut layout, now()).preset, Some(1));
        c.set_format("Week %V");
        assert_eq!(c.evaluate(&mut layout, now()).preset, None);
        assert!(!c.select_preset(PRESETS.len()));
    }

    #[test]
    fn two_line_formats_preview_both_lines_and_accept_edit_control_line_breaks() {
        let (mut c, _) = fresh();
        c.set_format("%H:%M\r\n%g%V.%u");
        assert_eq!(c.draft().format, "%H:%M\n%g%V.%u");
        assert!(!c.is_dirty(), "CRLF from the edit control is not a change");
        let eval = c.evaluate(&mut FakeLayout::default(), now());
        assert_eq!(
            eval.preview.lines().next(),
            Some("Now: 09:15 \u{21b5} 2642.2")
        );
        assert_eq!(eval.preset, Some(2));
    }

    #[test]
    fn invalid_formats_are_explained_beside_the_editor_and_located() {
        let (mut c, _) = fresh();
        let mut layout = FakeLayout::default();
        let cases = [
            (
                "%Q",
                "\u{201c}%Q\u{201d} is not a supported token",
                Some(0..2),
            ),
            ("%H%", "A % must be followed by a token", Some(2..3)),
            ("", "Enter a format, or choose a preset.", Some(0..0)),
            ("%H\n", "Each line needs some content", Some(4..4)),
            ("%H\n%M\n%S", "Use at most two lines.", Some(6..8)),
            ("a\tb", "Tabs and other control characters", Some(1..2)),
            (&"x".repeat(257), "at most 256 characters", None),
        ];
        for (text, message, selection) in cases {
            c.set_format(text);
            let eval = c.evaluate(&mut layout, now());
            let problem = eval.problems.format.as_ref().expect(text);
            assert!(problem.message.contains(message), "{text:?}: {problem:?}");
            if let Some(selection) = selection {
                assert_eq!(problem.selection, Some(selection), "{text:?}");
            }
            assert_eq!(eval.problems.first(), Some(Field::Format));
            assert_eq!(eval.preview, "Fix the format to see a preview.");
        }
        assert!(
            layout.calls.is_empty(),
            "no fit check for an unusable format"
        );
    }

    #[test]
    fn edit_selection_counts_crlf_and_utf16_units() {
        // "🕒" is two UTF-16 units; the line break is two in the edit control.
        assert_eq!(edit_selection("\u{1f552}\n%Q", 2..4), 4..6);
        assert_eq!(edit_selection("ab", 2..2), 2..2);
        assert_eq!(edit_selection("a\nb", 1..2), 1..3);
        assert_eq!(edit_selection("", 0..0), 0..0);
        assert_eq!(edit_selection("ab", 1..99), 1..2);
    }

    #[test]
    fn font_size_bounds_and_text_are_reported_beside_the_control() {
        let (mut c, _) = fresh();
        let mut layout = FakeLayout::default();
        for bad in ["", "abc", "5", "25", "-3", "99999999999999999999", "1.5"] {
            c.set_font_size(bad);
            let eval = c.evaluate(&mut layout, now());
            assert_eq!(
                eval.problems.font.as_deref(),
                Some("Font size must be between 6 and 24 pt."),
                "{bad:?}"
            );
            assert_eq!(eval.problems.first(), Some(Field::FontSize));
        }
        for good in ["6", "24", " 12 ", "09"] {
            c.set_font_size(good);
            assert!(
                c.evaluate(&mut layout, now()).problems.font.is_none(),
                "{good:?}"
            );
        }
        c.set_font_size("09");
        assert!(!c.is_dirty(), "09 is the same size as 9");
    }

    #[test]
    fn fit_is_checked_for_the_selected_displays_and_blocks_apply() {
        let (mut c, log) = fresh();
        let mut layout = FakeLayout {
            problems: vec![too_wide()],
            ..Default::default()
        };
        c.set_monitors(Monitors::All);
        c.set_font_size("20");
        let eval = c.evaluate(&mut layout, now());
        assert_eq!(
            layout.calls.last(),
            Some(&("09:15\n2642.2".to_owned(), 20, Monitors::All))
        );
        let fit = eval.problems.fit.as_ref().expect("fit problem");
        assert!(fit.contains("the primary taskbar"), "{fit}");
        assert!(
            fit.contains("needs 140 \u{d7} 32 px, 96 \u{d7} 40 px available"),
            "{fit}"
        );
        assert!(fit.contains("shorter format or a smaller font size"));
        assert_eq!(eval.problems.first(), Some(Field::Format));
        assert_eq!(c.apply(&mut layout, now(), false), ApplyOutcome::Invalid);
        assert!(
            log.entries().is_empty(),
            "nothing is saved while it does not fit"
        );
        assert_eq!(c.active().font_size_pt(), 9);
    }

    #[test]
    fn apply_persists_first_then_returns_the_value_and_cleans_the_draft() {
        let (mut c, log) = fresh();
        let mut layout = FakeLayout::default();
        c.set_format("%y%V.%u");
        c.set_font_size("12");
        c.set_monitors(Monitors::All);
        c.set_theme(Theme::Dark);
        assert!(c.is_dirty());
        let ApplyOutcome::Applied(value) = c.apply(&mut layout, now(), false) else {
            panic!("expected Applied");
        };
        assert_eq!(log.entries(), ["save"]);
        assert_eq!(value.format_source(), "%y%V.%u");
        assert_eq!(value.font_size_pt(), 12);
        assert_eq!(value.monitors(), Monitors::All);
        assert_eq!(value.theme(), Theme::Dark);
        assert_eq!(c.active(), &value);
        assert!(!c.is_dirty());
    }

    #[test]
    fn failed_save_keeps_live_settings_and_the_editable_draft() {
        let log = Log::default();
        let store = FakeStore::new(&log);
        let fail_save = store.fail_save.clone();
        fail_save.set(true);
        let mut c = controller(store);
        let mut layout = FakeLayout::default();
        c.set_font_size("14");
        c.set_format("%d/%m");
        match c.apply(&mut layout, now(), false) {
            ApplyOutcome::NotSaved(message) => {
                assert!(
                    message.contains("previous settings are preserved"),
                    "{message}"
                );
                assert!(
                    !message.contains("disk full"),
                    "internal detail leaked: {message}"
                );
            }
            other => panic!("expected NotSaved, got {other:?}"),
        }
        assert_eq!(c.active().font_size_pt(), 9);
        assert_eq!(c.active().format_source(), "%H:%M\n%g%V.%u");
        assert_eq!(c.draft().font_size, "14");
        assert_eq!(c.draft().format, "%d/%m");
        assert!(c.is_dirty());
        // The same draft can simply be applied again once saving works.
        fail_save.set(false);
        assert!(matches!(
            c.apply(&mut layout, now(), false),
            ApplyOutcome::Applied(_)
        ));
    }

    #[test]
    fn cancel_and_reset_are_predictable() {
        let (mut c, log) = fresh();
        let mut layout = FakeLayout::default();
        c.set_font_size("14");
        c.set_theme(Theme::Light);
        c.discard();
        assert_eq!(c.draft().font_size, "9");
        assert_eq!(c.draft().theme, Theme::System);
        assert!(!c.is_dirty());

        // Apply something, then stage defaults: live values stay until Apply.
        c.set_font_size("14");
        assert!(matches!(
            c.apply(&mut layout, now(), false),
            ApplyOutcome::Applied(_)
        ));
        c.set_monitors(Monitors::All);
        c.reset_to_defaults();
        assert_eq!(c.draft(), &Draft::from_preferences(&Preferences::default()));
        assert!(c.is_dirty());
        assert_eq!(c.active().font_size_pt(), 14);
        assert_eq!(log.entries(), ["save"], "reset alone must not write");
        // Cancel after reset drops the staged defaults.
        c.discard();
        assert_eq!(c.draft().font_size, "14");
        // Reset then Apply commits the defaults.
        c.reset_to_defaults();
        let ApplyOutcome::Applied(value) = c.apply(&mut layout, now(), false) else {
            panic!("expected Applied");
        };
        assert_eq!(value.font_size_pt(), 9);
        assert_eq!(log.entries(), ["save", "save"]);
        // Reset when already at defaults is not a change.
        c.reset_to_defaults();
        assert!(!c.is_dirty());
    }

    #[test]
    fn damaged_saved_data_needs_confirmation_and_is_preserved() {
        let log = Log::default();
        let mut store = FakeStore::new(&log);
        store.damaged = true;
        let mut c = controller(store);
        let mut layout = FakeLayout::default();
        assert!(c.notice().unwrap().contains("damaged"));
        assert!(!c.saving_blocked());
        c.set_font_size("11");
        assert_eq!(
            c.apply(&mut layout, now(), false),
            ApplyOutcome::RecoveryRequired
        );
        assert!(log.entries().is_empty(), "nothing is replaced unconfirmed");
        let ApplyOutcome::Applied(value) = c.apply(&mut layout, now(), true) else {
            panic!("expected recovery to apply");
        };
        assert_eq!(log.entries(), ["preserve", "save"]);
        assert_eq!(value.font_size_pt(), 11);
        assert!(c.notice().is_none());
    }

    #[test]
    fn a_newer_version_blocks_saving() {
        struct Newer;
        impl PreferencesStore for Newer {
            fn load(&self) -> Result<StoredPreferences, StoreError> {
                Err(StoreError::UnsupportedVersion(9))
            }
            fn save(&mut self, _: &StoredPreferences) -> Result<(), StoreError> {
                panic!("must not save");
            }
            fn preserve_for_recovery(&mut self) -> Result<(), StoreError> {
                panic!("must not preserve");
            }
        }
        let mut c = SettingsController::new(PreferenceSession::open(Newer));
        assert!(c.saving_blocked());
        assert!(c.notice().unwrap().contains("saving is disabled"));
        for recover in [false, true] {
            assert!(
                matches!(
                    c.apply(&mut FakeLayout::default(), now(), recover),
                    ApplyOutcome::NotSaved(_)
                ),
                "recover={recover}"
            );
        }
    }

    #[test]
    fn first_run_completes_once_and_is_best_effort() {
        let (mut c, log) = fresh();
        assert!(c.first_run());
        c.finish_first_run();
        assert!(!c.first_run());
        c.finish_first_run();
        assert_eq!(
            log.entries(),
            ["save"],
            "completion is written exactly once"
        );

        // A failing store leaves first-run pending without raising anything.
        let log = Log::default();
        let store = FakeStore::new(&log);
        store.fail_save.set(true);
        let mut c = controller(store);
        c.finish_first_run();
        assert!(c.first_run());
    }

    #[test]
    fn token_help_lists_every_supported_token() {
        for directive in clock_core::format::DIRECTIVES.chars() {
            let token = format!("%{directive}");
            assert!(TOKEN_HELP.contains(&token), "help is missing {token}");
        }
        assert!(TOKEN_HELP.contains("ISO week-year"));
        assert!(TOKEN_HELP.contains("Calendar year"));
    }

    #[test]
    fn dirty_tracks_each_field() {
        let (mut c, _) = fresh();
        c.set_monitors(Monitors::All);
        assert!(c.is_dirty());
        c.set_monitors(Monitors::Primary);
        assert!(!c.is_dirty());
        c.set_theme(Theme::Dark);
        assert!(c.is_dirty());
        c.set_theme(Theme::System);
        c.set_font_size("");
        assert!(c.is_dirty());
    }
}
