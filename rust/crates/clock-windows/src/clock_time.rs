//! One fresh OS-local time snapshot per update (R08).
//!
//! R06 requires each time event to carry a `DateTime<FixedOffset>` taken from the
//! operating system's current zone and DST rules *at that instant*; the runtime
//! never infers rules from an offset or extrapolates a previous reading. Windows
//! gives the UTC instant and the zone-resolved civil time for that same instant;
//! the offset is their difference, so DST and zone edits are honoured exactly.
use chrono::{DateTime, FixedOffset, NaiveDate};

/// Civil (calendar) fields as Windows reports them in a `SYSTEMTIME`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Civil {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}
impl Civil {
    fn seconds(self) -> Option<i64> {
        Some(
            NaiveDate::from_ymd_opt(self.year, self.month, self.day)?
                .and_hms_opt(self.hour, self.minute, self.second.min(59))?
                .and_utc()
                .timestamp(),
        )
    }
}

/// Windows' FILETIME epoch (1601-01-01) to Unix epoch, in 100 ns ticks.
pub const FILETIME_UNIX_EPOCH: i64 = 116_444_736_000_000_000;

/// Build the snapshot. `filetime` is the UTC instant in 100 ns ticks since 1601;
/// `utc` and `local` are that same instant as civil fields before/after the
/// zone conversion. Returns `None` for impossible inputs (never guesses).
pub fn local_snapshot(filetime: i64, utc: Civil, local: Civil) -> Option<DateTime<FixedOffset>> {
    let offset = local.seconds()?.checked_sub(utc.seconds()?)?;
    let zone = FixedOffset::east_opt(i32::try_from(offset).ok()?)?;
    let unix = filetime.checked_sub(FILETIME_UNIX_EPOCH)?;
    let seconds = unix.div_euclid(10_000_000);
    let nanos = u32::try_from(unix.rem_euclid(10_000_000) * 100).ok()?;
    Some(DateTime::from_timestamp(seconds, nanos)?.with_timezone(&zone))
}

#[cfg(windows)]
pub fn local_now() -> DateTime<FixedOffset> {
    use windows::Win32::{
        Foundation::{FILETIME, SYSTEMTIME},
        System::{
            SystemInformation::GetSystemTimePreciseAsFileTime,
            Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
        },
    };
    let civil = |t: SYSTEMTIME| Civil {
        year: i32::from(t.wYear),
        month: u32::from(t.wMonth),
        day: u32::from(t.wDay),
        hour: u32::from(t.wHour),
        minute: u32::from(t.wMinute),
        second: u32::from(t.wSecond),
    };
    // SAFETY: plain value conversions into live stack structures.
    let converted = unsafe {
        let ft: FILETIME = GetSystemTimePreciseAsFileTime();
        let ticks = (i64::from(ft.dwHighDateTime) << 32) | i64::from(ft.dwLowDateTime);
        let mut utc = SYSTEMTIME::default();
        let mut local = SYSTEMTIME::default();
        (FileTimeToSystemTime(&ft, &mut utc).is_ok()
            && SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_ok())
        .then(|| local_snapshot(ticks, civil(utc), civil(local)))
        .flatten()
    };
    // A failing zone conversion degrades to UTC rather than a wrong local time.
    converted.unwrap_or_else(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        DateTime::from_timestamp(now.as_secs() as i64, now.subsec_nanos())
            .unwrap_or_default()
            .fixed_offset()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    fn civil(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Civil {
        Civil {
            year: y,
            month: mo,
            day: d,
            hour: h,
            minute: mi,
            second: s,
        }
    }
    fn ticks(unix_seconds: i64, sub_100ns: i64) -> i64 {
        FILETIME_UNIX_EPOCH + unix_seconds * 10_000_000 + sub_100ns
    }

    #[test]
    fn offset_is_the_difference_between_the_two_civil_readings() {
        // 2026-07-14 10:00:00 UTC is 12:00 in Amsterdam (CEST, +02:00).
        let utc = DateTime::parse_from_rfc3339("2026-07-14T10:00:00+00:00").unwrap();
        let snapshot = local_snapshot(
            ticks(utc.timestamp(), 2_500_000),
            civil(2026, 7, 14, 10, 0, 0),
            civil(2026, 7, 14, 12, 0, 0),
        )
        .unwrap();
        assert_eq!(snapshot.offset().local_minus_utc(), 7200);
        assert_eq!(
            (snapshot.hour(), snapshot.minute(), snapshot.second()),
            (12, 0, 0)
        );
        assert_eq!(snapshot.nanosecond(), 250_000_000);
        assert_eq!(snapshot.timestamp(), utc.timestamp());
    }

    #[test]
    fn dst_transitions_use_the_zone_rules_in_force_at_that_instant() {
        // Spring forward in New York: 06:59:59 UTC is 01:59:59 EST, 07:00:00 UTC is 03:00:00 EDT.
        let before = DateTime::parse_from_rfc3339("2026-03-08T06:59:59+00:00").unwrap();
        let after = DateTime::parse_from_rfc3339("2026-03-08T07:00:00+00:00").unwrap();
        let a = local_snapshot(
            ticks(before.timestamp(), 0),
            civil(2026, 3, 8, 6, 59, 59),
            civil(2026, 3, 8, 1, 59, 59),
        )
        .unwrap();
        let b = local_snapshot(
            ticks(after.timestamp(), 0),
            civil(2026, 3, 8, 7, 0, 0),
            civil(2026, 3, 8, 3, 0, 0),
        )
        .unwrap();
        assert_eq!(a.offset().local_minus_utc(), -5 * 3600);
        assert_eq!(b.offset().local_minus_utc(), -4 * 3600);
        assert_eq!(b.timestamp() - a.timestamp(), 1, "one real second passed");
    }

    #[test]
    fn fractional_and_negative_offsets_and_date_rollover() {
        // India (+05:30) at 20:00 UTC is already the next local day.
        let utc = DateTime::parse_from_rfc3339("2026-12-31T20:00:00+00:00").unwrap();
        let s = local_snapshot(
            ticks(utc.timestamp(), 0),
            civil(2026, 12, 31, 20, 0, 0),
            civil(2027, 1, 1, 1, 30, 0),
        )
        .unwrap();
        assert_eq!(s.offset().local_minus_utc(), 19_800);
        assert_eq!((s.year(), s.month(), s.day()), (2027, 1, 1));
        let west = local_snapshot(
            ticks(utc.timestamp(), 0),
            civil(2026, 12, 31, 20, 0, 0),
            civil(2026, 12, 31, 11, 30, 0),
        )
        .unwrap();
        assert_eq!(west.offset().local_minus_utc(), -8 * 3600 - 1800);
    }

    #[test]
    fn impossible_inputs_are_none_not_guesses() {
        let ok = civil(2026, 1, 1, 0, 0, 0);
        assert!(local_snapshot(ticks(0, 0), civil(2026, 13, 1, 0, 0, 0), ok).is_none());
        assert!(local_snapshot(ticks(0, 0), ok, civil(2026, 2, 30, 0, 0, 0)).is_none());
        // A zone conversion off by more than a day is rejected.
        assert!(
            local_snapshot(
                ticks(1_700_000_000, 0),
                civil(2026, 1, 1, 0, 0, 0),
                civil(2026, 1, 3, 0, 0, 0)
            )
            .is_none()
        );
    }
}
