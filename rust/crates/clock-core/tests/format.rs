use chrono::{DateTime, FixedOffset, NaiveDateTime};
use clock_core::{Cadence, CompiledFormat, FormatErrorKind, format::*};
use proptest::prelude::*;

fn snapshot(value: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(value).unwrap()
}
fn render(format: &str, value: &str) -> Vec<String> {
    CompiledFormat::parse(format)
        .unwrap()
        .render(snapshot(value))
        .unwrap()
        .lines
}

#[test]
fn iso_and_calendar_presets_at_year_boundaries() {
    for (date, iso, calendar) in [
        ("2026-10-13", "2642.2", "2642.2"),
        ("2027-01-01", "2653.5", "2753.5"),
        ("2027-01-04", "2701.1", "2701.1"),
        ("2025-12-29", "2601.1", "2501.1"),
        ("2027-01-03", "2653.7", "2753.7"),
    ] {
        let time = format!("{date}T23:59:58+01:00");
        assert_eq!(render(PRESETS[0].format, &time), [iso]);
        assert_eq!(render(PRESETS[1].format, &time), [calendar]);
        assert_eq!(render(PRESETS[2].format, &time), ["23:59", iso]);
    }
}

#[test]
fn every_directive_english_names_leap_day_and_literals() {
    assert_eq!(
        render(
            "%Y %y %G %g %m %d %H %I %M %S %p %V %u %a %A %b %B %% 🕒日",
            "2024-02-29T00:07:09+01:00"
        ),
        ["2024 24 2024 24 02 29 00 12 07 09 AM 09 4 Thu Thursday Feb February % 🕒日"]
    );
    assert_eq!(render("%I %p", "2024-02-29T13:00:00+01:00"), ["01 PM"]);
    assert_eq!(
        render("%H:%M\r\n%g%V.%u", "2027-01-01T00:00:00+01:00"),
        ["00:00", "2653.5"]
    );
    assert_eq!(render("%%S", "2027-01-01T00:00:00+01:00"), ["%S"]);
}

#[test]
fn cadence_and_preview_use_supplied_snapshots() {
    for value in ["%H:%M", "%Y-%m-%d", "%%S", "seconds"] {
        assert_eq!(
            CompiledFormat::parse(value).unwrap().cadence(),
            Cadence::Minute
        );
    }
    assert_eq!(
        CompiledFormat::parse("%H\n%S").unwrap().cadence(),
        Cadence::Second
    );
    // Explicit Amsterdam snapshots around both DST transitions. Fixed offsets
    // describe these instants only; the formatter never derives zone rules.
    let samples = [
        snapshot("2026-03-29T01:59:59+01:00"),
        snapshot("2026-03-29T03:00:00+02:00"),
        snapshot("2026-10-25T02:59:59+02:00"),
        snapshot("2026-10-25T02:00:00+01:00"),
    ];
    let previews = preview("%H:%M:%S\n%Y-%m-%d", &samples).unwrap();
    for (result, time) in previews
        .into_iter()
        .zip(["01:59:59", "03:00:00", "02:59:59", "02:00:00"])
    {
        assert_eq!(result.unwrap().lines[0], time);
    }
    assert!(preview("%H", &[]).unwrap().is_empty());
}

#[test]
fn malformed_formats_have_original_character_spans() {
    for (value, kind, span) in [
        ("", FormatErrorKind::EmptyLine, 0..0),
        ("\nx", FormatErrorKind::EmptyLine, 0..0),
        ("x\n", FormatErrorKind::EmptyLine, 2..2),
        ("x\n\ny", FormatErrorKind::TooManyLines, 2..3),
        ("🕒\r\n%Q", FormatErrorKind::UnsupportedDirective, 3..5),
        ("🕒%", FormatErrorKind::IncompleteDirective, 1..2),
        ("x\r", FormatErrorKind::ControlCharacter, 1..2),
        ("x\t", FormatErrorKind::ControlCharacter, 1..2),
        ("x\0", FormatErrorKind::ControlCharacter, 1..2),
        ("x\u{85}", FormatErrorKind::ControlCharacter, 1..2),
    ] {
        let error = CompiledFormat::parse(value).unwrap_err();
        assert_eq!((error.kind, error.span), (kind, span), "{value:?}");
    }
    for token in [
        "%n", "%t", "%-d", "%_H", "%0H", "%:z", "%3S", "%f", "%z", "%Z", "%W", "%U", "%s", "%c",
        "%D", "%e",
    ] {
        let error = CompiledFormat::parse(token).unwrap_err();
        assert_eq!(error.kind, FormatErrorKind::UnsupportedDirective, "{token}");
        assert_eq!(error.span, 0..2);
    }
}

#[test]
fn exact_input_and_output_bounds() {
    let sample = snapshot("2026-09-30T00:00:00+02:00");
    assert!(
        CompiledFormat::parse(&"日".repeat(256))
            .unwrap()
            .render(sample)
            .is_ok()
    );
    let error = CompiledFormat::parse(&"日".repeat(257)).unwrap_err();
    assert_eq!(
        (error.kind, error.span),
        (FormatErrorKind::InputTooLong, 256..257)
    );
    assert!(CompiledFormat::parse(&format!("{}\r\nx", "日".repeat(253))).is_ok());
    assert!(CompiledFormat::parse(&format!("{}\r\nx", "日".repeat(254))).is_err());
    // September is nine scalars: 56*9 + 8 literals = exactly 512.
    let exact = format!("{}{}", "%B".repeat(56), "日".repeat(8));
    let text = CompiledFormat::parse(&exact)
        .unwrap()
        .render(sample)
        .unwrap();
    assert_eq!(text.lines[0].chars().count(), 512);
    let error = CompiledFormat::parse(&(exact + "日"))
        .unwrap()
        .render(sample)
        .unwrap_err();
    assert_eq!(error.kind, FormatErrorKind::OutputTooLong);
    assert_eq!(error.span, 0..121);
    let two_lines = format!("{}\nx", "%B".repeat(56) + &"日".repeat(6));
    let text = CompiledFormat::parse(&two_lines)
        .unwrap()
        .render(sample)
        .unwrap();
    assert_eq!(
        text.lines
            .iter()
            .map(|line| line.chars().count())
            .sum::<usize>()
            + 1,
        512
    );
    assert_eq!(
        CompiledFormat::parse(&(two_lines + "日"))
            .unwrap()
            .render(sample)
            .unwrap_err()
            .kind,
        FormatErrorKind::OutputTooLong
    );
    let over = CompiledFormat::parse(&"%B".repeat(128)).unwrap();
    assert_eq!(
        over.render(sample).unwrap_err().kind,
        FormatErrorKind::OutputTooLong
    );
}

#[test]
fn chrono_extreme_offset_is_an_error_not_a_panic() {
    let all_tokens: String = DIRECTIVES.chars().map(|c| format!("%{c}")).collect();
    for endpoint in [NaiveDateTime::MIN, NaiveDateTime::MAX] {
        let sample =
            DateTime::from_naive_utc_and_offset(endpoint, FixedOffset::east_opt(0).unwrap());
        assert!(
            CompiledFormat::parse(&all_tokens)
                .unwrap()
                .render(sample)
                .is_ok()
        );
    }
    let sample = DateTime::from_naive_utc_and_offset(
        NaiveDateTime::MAX,
        FixedOffset::east_opt(3600).unwrap(),
    );
    assert_eq!(
        CompiledFormat::parse("%Y")
            .unwrap()
            .render(sample)
            .unwrap_err()
            .kind,
        FormatErrorKind::SnapshotOutOfRange
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]
    #[test]
    fn arbitrary_input_is_bounded(chars in prop::collection::vec(any::<char>(), 0..600)) {
        let input: String = chars.iter().collect();
        match CompiledFormat::parse(&input) {
            Ok(compiled) => {
                if let Ok(text) = compiled.render(snapshot("2027-01-01T23:59:59+01:00")) {
                    prop_assert!((1..=2).contains(&text.lines.len()));
                    prop_assert!(text.lines.iter().all(|line| !line.is_empty()));
                    prop_assert!(text.lines.iter().map(|line| line.chars().count()).sum::<usize>() + text.lines.len() - 1 <= MAX_RENDERED_SCALARS);
                }
            },
            Err(error) => {
                prop_assert!(error.span.start <= error.span.end);
                prop_assert!(error.span.end <= chars.len());
            }
        }
    }
    #[test]
    fn generated_allowed_tokens_match_chrono(
        tokens in prop::collection::vec(prop::sample::select(DIRECTIVES.chars().collect::<Vec<_>>()), 1..129),
        seconds in -2_000_000_000i64..2_000_000_000,
        offset in -86399i32..86400,
    ) {
        let input: String = tokens.iter().map(|c| format!("%{c}")).collect();
        let sample = DateTime::from_timestamp(seconds, 0).unwrap().with_timezone(&FixedOffset::east_opt(offset).unwrap());
        let expected = sample.format(&input).to_string();
        let compiled = CompiledFormat::parse(&input).unwrap();
        match compiled.render(sample) {
            Ok(text) => prop_assert_eq!(&text.lines[0], &expected),
            Err(error) => {
                prop_assert_eq!(error.kind, FormatErrorKind::OutputTooLong);
                prop_assert!(expected.chars().count() > 512);
            }
        }
        prop_assert_eq!(compiled.cadence(), if tokens.contains(&'S') { Cadence::Second } else { Cadence::Minute });
    }
}
