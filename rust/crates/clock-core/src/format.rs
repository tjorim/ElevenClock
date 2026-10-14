//! Validated, owned datetime formats. Names are English; offsets are snapshots,
//! not timezone rules. No clock access or expression evaluation is performed.
use crate::{Cadence, ClockText};
use chrono::{DateTime, FixedOffset, TimeDelta, format::Item, format::StrftimeItems};
use std::{fmt, ops::Range};

pub const MAX_INPUT_SCALARS: usize = 256;
pub const MAX_RENDERED_SCALARS: usize = 512;
/// The complete directive allowlist, including the literal-percent escape.
pub const DIRECTIVES: &str = "YyGgmdHIMSpVuaAbB%";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatErrorKind {
    InputTooLong,
    EmptyLine,
    TooManyLines,
    ControlCharacter,
    IncompleteDirective,
    UnsupportedDirective,
    OutputTooLong,
    SnapshotOutOfRange,
}

/// Half-open Unicode scalar indices in the ORIGINAL input (not UTF-8 bytes).
/// CRLF occupies two indices. Render failures cover the entire original input;
/// an empty input has span 0..0. Length errors point at the first excess scalar.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatError {
    pub kind: FormatErrorKind,
    pub span: Range<usize>,
}
impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} at characters {}..{}",
            self.kind, self.span.start, self.span.end
        )
    }
}
impl std::error::Error for FormatError {}

#[derive(Clone, Debug)]
pub struct CompiledFormat {
    items: Vec<Item<'static>>,
    cadence: Cadence,
    input_len: usize,
}
impl CompiledFormat {
    pub fn parse(input: &str) -> Result<Self, FormatError> {
        let fail = |kind, span| FormatError { kind, span };
        // Bound work before collecting arbitrary input.
        let chars: Vec<_> = input.chars().take(MAX_INPUT_SCALARS + 1).collect();
        if chars.len() > MAX_INPUT_SCALARS {
            return Err(fail(FormatErrorKind::InputTooLong, 256..257));
        }
        let mut items = Vec::new();
        let mut cadence = Cadence::Minute;
        let mut i = 0;
        let mut line_start = 0;
        let mut has_content = false;
        let mut newline_seen = false;
        while i < chars.len() {
            let c = chars[i];
            let crlf = c == '\r' && chars.get(i + 1) == Some(&'\n');
            if c == '\n' || crlf {
                let end = i + if crlf { 2 } else { 1 };
                if newline_seen {
                    return Err(fail(FormatErrorKind::TooManyLines, i..end));
                }
                if !has_content {
                    return Err(fail(FormatErrorKind::EmptyLine, line_start..i));
                }
                items.push(Item::OwnedLiteral("\n".into()));
                newline_seen = true;
                has_content = false;
                line_start = end;
                i = end;
                continue;
            }
            if c.is_control() {
                return Err(fail(FormatErrorKind::ControlCharacter, i..i + 1));
            }
            if c == '%' {
                let Some(&directive) = chars.get(i + 1) else {
                    return Err(fail(FormatErrorKind::IncompleteDirective, i..i + 1));
                };
                if !DIRECTIVES.contains(directive) {
                    return Err(fail(FormatErrorKind::UnsupportedDirective, i..i + 2));
                }
                if directive == 'S' {
                    cadence = Cadence::Second;
                }
                let token = format!("%{directive}");
                items.extend(StrftimeItems::new(&token).map(|item| item.to_owned()));
                i += 2;
            } else {
                // Always literals, even if they resemble HTML or expressions.
                items.push(Item::OwnedLiteral(c.to_string().into()));
                i += 1;
            }
            has_content = true;
        }
        if !has_content {
            return Err(fail(FormatErrorKind::EmptyLine, line_start..chars.len()));
        }
        Ok(Self {
            items,
            cadence,
            input_len: chars.len(),
        })
    }

    pub const fn cadence(&self) -> Cadence {
        self.cadence
    }

    /// Both lines use exactly this supplied snapshot. Output bounds include the
    /// normalized newline. Failure returns no partial/truncated clock text.
    pub fn render(&self, snapshot: DateTime<FixedOffset>) -> Result<ClockText, FormatError> {
        let fail = |kind| FormatError {
            kind,
            span: 0..self.input_len,
        };
        // DateTime::naive_local can panic at chrono's range endpoints. Check it.
        let local = snapshot
            .naive_utc()
            .checked_add_signed(TimeDelta::seconds(i64::from(
                snapshot.offset().local_minus_utc(),
            )))
            .ok_or_else(|| fail(FormatErrorKind::SnapshotOutOfRange))?;
        let mut output = BoundedOutput::default();
        use std::fmt::Write;
        write!(
            &mut output,
            "{}",
            local.format_with_items(self.items.iter())
        )
        .map_err(|_| fail(FormatErrorKind::OutputTooLong))?;
        Ok(ClockText {
            lines: output.text.split('\n').map(str::to_owned).collect(),
        })
    }

    /// Preview user-supplied samples without reading system time or zone rules.
    pub fn preview(
        &self,
        snapshots: &[DateTime<FixedOffset>],
    ) -> Vec<Result<ClockText, FormatError>> {
        snapshots
            .iter()
            .map(|snapshot| self.render(*snapshot))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatPreset {
    pub name: &'static str,
    pub format: &'static str,
}
pub const PRESETS: [FormatPreset; 3] = [
    FormatPreset {
        name: "ISO week date",
        format: "%g%V.%u",
    },
    FormatPreset {
        name: "Calendar year + ISO week",
        format: "%y%V.%u",
    },
    FormatPreset {
        name: "Time + ISO week date",
        format: "%H:%M\n%g%V.%u",
    },
];

/// Parse and preview an editor value using caller-selected timestamps.
/// ```
/// use clock_core::format::preview;
/// let sample = chrono::DateTime::parse_from_rfc3339("2027-01-01T12:34:00+01:00").unwrap();
/// assert_eq!(preview("%g%V.%u", &[sample]).unwrap()[0].as_ref().unwrap().lines, ["2653.5"]);
/// ```
pub fn preview(
    input: &str,
    snapshots: &[DateTime<FixedOffset>],
) -> Result<Vec<Result<ClockText, FormatError>>, FormatError> {
    Ok(CompiledFormat::parse(input)?.preview(snapshots))
}

/// A single formatting pass with a checked sink, including the line separator.
#[derive(Default)]
struct BoundedOutput {
    text: String,
    scalars: usize,
}
impl fmt::Write for BoundedOutput {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let count = value.chars().count();
        if self.scalars + count > MAX_RENDERED_SCALARS {
            return Err(fmt::Error);
        }
        self.scalars += count;
        self.text.push_str(value);
        Ok(())
    }
}
