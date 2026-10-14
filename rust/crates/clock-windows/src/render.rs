//! Clock text layout: DPI conversion, fit/overflow decisions and (on Windows) the
//! DirectWrite/Direct2D surface that paints them.
//!
//! Everything here except `native` is portable and unit tested. Layout works in
//! signed physical pixels: the Direct2D target always runs at 96 DPI over a
//! physical-pixel HWND, and point sizes are converted exactly once, in
//! `font_size_px`, using the DPI that R02 measured for that taskbar. There is no
//! other scale factor, so mixed-DPI monitors cannot disagree about units.
use clock_core::{ClockText, CompiledFormat, PhysicalRect};
use std::{
    collections::BTreeSet,
    fmt,
    hash::{Hash, Hasher},
};

pub const BASE_DPI: u32 = 96;
const POINTS_PER_INCH: f64 = 72.0;
/// Space kept free at each side of the text, in 96-DPI units. Never negative.
pub const HORIZONTAL_PADDING_DIP: i32 = 4;

/// Font height in physical pixels for a configured point size at `dpi`.
/// `None` for a zero size or DPI, which are never valid shell data.
pub fn font_size_px(font_pt: u8, dpi: u32) -> Option<f32> {
    if font_pt == 0 || dpi == 0 {
        return None;
    }
    Some((f64::from(font_pt) * f64::from(dpi) / POINTS_PER_INCH) as f32)
}
/// Round-to-nearest conversion of a 96-DPI length, saturating at the i32 range.
pub fn scale_dip(dip: i32, dpi: u32) -> i32 {
    let scaled =
        (i64::from(dip) * i64::from(dpi) + i64::from(BASE_DPI) / 2).div_euclid(i64::from(BASE_DPI));
    scaled.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}
pub fn padding_px(dpi: u32) -> i32 {
    scale_dip(HORIZONTAL_PADDING_DIP, dpi)
}

/// A width/height in physical pixels. Components may exceed the region; they are
/// never negative.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PixelSize {
    pub width: i32,
    pub height: i32,
}
impl PixelSize {
    pub fn of(rect: PhysicalRect) -> Self {
        Self {
            width: rect.width(),
            height: rect.height(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderError {
    /// Zero DPI or font size; discovery/preference validation should prevent it.
    InvalidScale,
    /// A measurement was negative, NaN or infinite.
    InvalidMeasurement,
    /// A native call failed; `code` is the HRESULT.
    Native { operation: &'static str, code: i32 },
}
impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScale => f.write_str("Invalid DPI or font size"),
            Self::InvalidMeasurement => f.write_str("Text measurement was not a finite size"),
            Self::Native { operation, code } => {
                write!(f, "{operation} failed: 0x{:08X}", *code as u32)
            }
        }
    }
}
impl std::error::Error for RenderError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub width: f32,
    pub height: f32,
}
/// Measures one line of text in the exact font the surface will paint with.
pub trait TextMeasurer {
    fn measure_line(&mut self, line: &str) -> Result<Extent, RenderError>;
}

/// The text does not fit its verified region. The overlay for that monitor is
/// hidden so the native clock stays visible; text is never clipped or wrapped,
/// and the region is never widened over the bell/tray.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Overflow {
    /// Required width includes both paddings.
    pub required: PixelSize,
    pub available: PixelSize,
}
impl Overflow {
    pub fn width_exceeded(&self) -> bool {
        self.required.width > self.available.width
    }
    pub fn height_exceeded(&self) -> bool {
        self.required.height > self.available.height
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// Font height in physical pixels (see `font_size_px`).
    pub font_px: f32,
    pub padding_px: i32,
    pub region: PixelSize,
    pub required: PixelSize,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fit {
    Fits(Layout),
    Overflow(Overflow),
}

/// Every distinct line the format can produce over a window of dates and times
/// that includes month/day/year boundaries, a leap day and ISO week 53. Fit is
/// decided against all of them so a format that is fine today cannot spill over
/// the notification area in September, on a leap day or at New Year.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorstCase {
    lines: Vec<BTreeSet<String>>,
    fingerprint: u64,
}
impl WorstCase {
    /// First and last dates sampled; 2026 is a 53-week ISO year and 2028 is leap.
    const FIRST_YEAR: i32 = 2026;
    const LAST_YEAR: i32 = 2028;
    /// Digit-rich and AM/PM-spanning wall times.
    const TIMES: [(u32, u32, u32); 5] = [
        (0, 0, 0),
        (8, 8, 8),
        (11, 11, 11),
        (20, 47, 29),
        (23, 59, 59),
    ];

    pub fn for_format(format: &CompiledFormat) -> Self {
        use chrono::{FixedOffset, NaiveDate, TimeZone};
        let offset = FixedOffset::east_opt(0).expect("zero offset");
        let first = NaiveDate::from_ymd_opt(Self::FIRST_YEAR, 1, 1).expect("valid date");
        let last = NaiveDate::from_ymd_opt(Self::LAST_YEAR, 12, 31).expect("valid date");
        let mut lines: Vec<BTreeSet<String>> = Vec::new();
        for date in first.iter_days().take_while(|date| *date <= last) {
            for (hour, minute, second) in Self::TIMES {
                let Some(naive) = date.and_hms_opt(hour, minute, second) else {
                    continue;
                };
                let Some(snapshot) = offset.from_local_datetime(&naive).single() else {
                    continue;
                };
                let Ok(text) = format.render(snapshot) else {
                    continue;
                };
                if lines.len() < text.lines.len() {
                    lines.resize_with(text.lines.len(), BTreeSet::new);
                }
                for (index, line) in text.lines.into_iter().enumerate() {
                    lines[index].insert(line);
                }
            }
        }
        Self::from_sets(lines)
    }
    /// For callers and tests that already know the candidate lines.
    pub fn from_lines<I, L>(lines: I) -> Self
    where
        I: IntoIterator<Item = L>,
        L: IntoIterator<Item = String>,
    {
        Self::from_sets(
            lines
                .into_iter()
                .map(|line| line.into_iter().collect())
                .collect(),
        )
    }
    fn from_sets(lines: Vec<BTreeSet<String>>) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        lines.hash(&mut hasher);
        Self {
            lines,
            fingerprint: hasher.finish(),
        }
    }
    /// Identity used by surfaces to reuse a previous worst-case verdict.
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }
    pub fn candidates(&self, line_index: usize) -> impl Iterator<Item = &str> {
        self.lines
            .get(line_index)
            .into_iter()
            .flat_map(|set| set.iter().map(String::as_str))
    }
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
}

fn ceil_px(value: f32) -> Result<i32, RenderError> {
    if !value.is_finite() || value < 0.0 {
        return Err(RenderError::InvalidMeasurement);
    }
    Ok(value.ceil().min(i32::MAX as f32) as i32)
}

/// Measure the current text and every worst-case candidate. Lines are laid out
/// independently, so the requirement is the widest candidate and, per line, the
/// tallest candidate summed over lines.
pub fn required_size<M: TextMeasurer>(
    measurer: &mut M,
    current: &ClockText,
    worst_case: &WorstCase,
) -> Result<PixelSize, RenderError> {
    let line_count = current.lines.len().max(worst_case.line_count());
    let (mut width, mut height) = (0i32, 0i32);
    for index in 0..line_count {
        let mut line_height = 0i32;
        let current_line = current.lines.get(index).map(String::as_str);
        for line in current_line.into_iter().chain(worst_case.candidates(index)) {
            let extent = measurer.measure_line(line)?;
            width = width.max(ceil_px(extent.width)?);
            line_height = line_height.max(ceil_px(extent.height)?);
        }
        height = height.saturating_add(line_height);
    }
    Ok(PixelSize { width, height })
}

/// Decide whether `current` and all worst-case lines fit `region`.
pub fn plan_fit<M: TextMeasurer>(
    measurer: &mut M,
    current: &ClockText,
    worst_case: &WorstCase,
    region: PixelSize,
    padding_px: i32,
    font_px: f32,
) -> Result<Fit, RenderError> {
    let padding_px = padding_px.max(0);
    let text = required_size(measurer, current, worst_case)?;
    let required = PixelSize {
        width: text.width.saturating_add(padding_px.saturating_mul(2)),
        height: text.height,
    };
    if required.width > region.width || required.height > region.height {
        return Ok(Fit::Overflow(Overflow {
            required,
            available: region,
        }));
    }
    Ok(Fit::Fits(Layout {
        font_px,
        padding_px,
        region,
        required,
    }))
}

#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::{ClockSurface, PaintOutcome, PaintRequest, Renderer};

#[cfg(test)]
mod tests {
    use super::*;
    use clock_core::Preferences;

    /// 7 px per scalar and a 16 px line, like a small UI font at 100%.
    struct Fixed;
    impl TextMeasurer for Fixed {
        fn measure_line(&mut self, line: &str) -> Result<Extent, RenderError> {
            Ok(Extent {
                width: 7.0 * line.chars().count() as f32,
                height: 16.0,
            })
        }
    }
    fn text(lines: &[&str]) -> ClockText {
        ClockText {
            lines: lines.iter().map(|line| (*line).into()).collect(),
        }
    }
    fn worst(format: &str) -> WorstCase {
        let prefs = Preferences {
            format: format.into(),
            ..Preferences::default()
        }
        .validate()
        .unwrap();
        WorstCase::for_format(prefs.format())
    }
    fn fit(current: &[&str], worst: &WorstCase, width: i32, height: i32) -> Fit {
        plan_fit(
            &mut Fixed,
            &text(current),
            worst,
            PixelSize { width, height },
            4,
            12.0,
        )
        .unwrap()
    }

    #[test]
    fn point_sizes_convert_exactly_at_supported_scales() {
        for (dpi, px) in [(96, 12.0), (120, 15.0), (144, 18.0), (192, 24.0)] {
            assert_eq!(font_size_px(9, dpi), Some(px), "{dpi} DPI");
        }
        assert_eq!(font_size_px(24, 192), Some(64.0));
        assert_eq!(font_size_px(9, 0), None);
        assert_eq!(font_size_px(0, 96), None);
    }
    #[test]
    fn dip_scaling_rounds_and_saturates() {
        assert_eq!(scale_dip(4, 96), 4);
        assert_eq!(scale_dip(4, 120), 5);
        assert_eq!(scale_dip(4, 144), 6);
        assert_eq!(scale_dip(4, 192), 8);
        assert_eq!(scale_dip(-4, 120), -5);
        assert_eq!(scale_dip(i32::MAX, u32::MAX), i32::MAX);
        assert_eq!(scale_dip(i32::MIN, u32::MAX), i32::MIN);
    }
    #[test]
    fn region_size_preserves_negative_monitor_coordinates() {
        let secondary = PhysicalRect::new(-1920 + 910, 1032, 74, 48).unwrap();
        assert_eq!(
            PixelSize::of(secondary),
            PixelSize {
                width: 74,
                height: 48
            }
        );
    }
    #[test]
    fn worst_case_covers_months_week_53_and_leap_day() {
        let months = worst("%B");
        let names: Vec<_> = months.candidates(0).collect();
        assert_eq!(names.len(), 12);
        assert!(names.contains(&"September"));

        let week = worst("%G-W%V");
        assert!(week.candidates(0).any(|line| line == "2026-W53"));
        // The ISO year differs from the calendar year around New Year.
        assert!(week.candidates(0).any(|line| line == "2027-W01"));

        let day = worst("%m-%d");
        assert!(day.candidates(0).any(|line| line == "02-29"));

        let two = worst("%H:%M\n%a");
        assert_eq!(two.line_count(), 2);
        assert_eq!(two.candidates(1).count(), 7);
    }
    #[test]
    fn worst_case_is_deterministic_and_content_addressed() {
        assert_eq!(worst("%B").fingerprint(), worst("%B").fingerprint());
        assert_ne!(worst("%B").fingerprint(), worst("%b").fingerprint());
    }
    #[test]
    fn default_two_line_text_fits_a_typical_clock_region() {
        let case = worst(&Preferences::default().format);
        match fit(&["14:35", "2642.2"], &case, 74, 48) {
            Fit::Fits(layout) => {
                assert_eq!(layout.required.height, 32);
                assert_eq!(layout.padding_px, 4);
            }
            other => panic!("expected fit, got {other:?}"),
        }
    }
    #[test]
    fn current_text_that_is_too_wide_overflows_instead_of_truncating() {
        let case = WorstCase::from_lines([["Wednesday 30 September 2026".to_string()]]);
        let Fit::Overflow(overflow) = fit(&["Wednesday 30 September 2026"], &case, 74, 48) else {
            panic!("expected overflow");
        };
        assert!(overflow.width_exceeded());
        assert!(!overflow.height_exceeded());
        assert_eq!(overflow.available.width, 74);
    }
    #[test]
    fn future_text_overflows_even_when_current_text_fits() {
        // In a 60 px region "May" fits but "September" (9 scalars) cannot.
        let case = worst("%B");
        assert!(matches!(
            fit(
                &["May"],
                &WorstCase::from_lines([["May".to_string()]]),
                74,
                48
            ),
            Fit::Fits(_)
        ));
        let Fit::Overflow(overflow) = fit(&["May"], &case, 60, 48) else {
            panic!("September must overflow");
        };
        assert_eq!(overflow.required.width, 7 * 9 + 8);
    }
    #[test]
    fn extra_lines_overflow_the_height_only() {
        let case = WorstCase::from_lines([
            ["12:00".to_string()],
            ["Wed".to_string()],
            ["30".to_string()],
        ]);
        let Fit::Overflow(overflow) = fit(&["12:00", "Wed", "30"], &case, 100, 40) else {
            panic!("three lines cannot fit 40 px");
        };
        assert!(overflow.height_exceeded());
        assert!(!overflow.width_exceeded());
        assert_eq!(overflow.required.height, 48);
    }
    #[test]
    fn exact_fit_is_accepted_and_one_pixel_less_is_not() {
        let case = WorstCase::from_lines([["abcd".to_string()]]);
        assert!(matches!(fit(&["abcd"], &case, 28 + 8, 16), Fit::Fits(_)));
        assert!(matches!(
            fit(&["abcd"], &case, 28 + 7, 16),
            Fit::Overflow(_)
        ));
        assert!(matches!(
            fit(&["abcd"], &case, 28 + 8, 15),
            Fit::Overflow(_)
        ));
    }
    #[test]
    fn invalid_measurements_are_errors_not_zero_sized_text() {
        struct Bad(f32);
        impl TextMeasurer for Bad {
            fn measure_line(&mut self, _: &str) -> Result<Extent, RenderError> {
                Ok(Extent {
                    width: self.0,
                    height: 16.0,
                })
            }
        }
        for value in [f32::NAN, f32::INFINITY, -1.0] {
            let result = required_size(
                &mut Bad(value),
                &text(&["x"]),
                &WorstCase::from_lines([Vec::<String>::new()]),
            );
            assert_eq!(result, Err(RenderError::InvalidMeasurement));
        }
    }
    #[test]
    fn measurement_errors_propagate() {
        struct Failing;
        impl TextMeasurer for Failing {
            fn measure_line(&mut self, _: &str) -> Result<Extent, RenderError> {
                Err(RenderError::Native {
                    operation: "CreateTextLayout",
                    code: 0x8899000Cu32 as i32,
                })
            }
        }
        let error = plan_fit(
            &mut Failing,
            &text(&["x"]),
            &WorstCase::from_lines([Vec::<String>::new()]),
            PixelSize {
                width: 10,
                height: 10,
            },
            0,
            12.0,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "CreateTextLayout failed: 0x8899000C");
    }
}
