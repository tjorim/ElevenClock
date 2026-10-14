//! Wall snapshots choose alignment; the runtime waits using a monotonic timer.
use crate::Cadence;
use chrono::{DateTime, FixedOffset, Timelike};
use std::time::Duration;

/// Strictly positive wait to the next local second/minute boundary. Date-only
/// formats use Minute. Each wake/resume must supply a fresh snapshot; never add
/// this duration to the previous wall time. Leap seconds wake at the next second.
pub fn next_delay(snapshot: DateTime<FixedOffset>, cadence: Cadence) -> Duration {
    let nanos = u64::from(snapshot.nanosecond() % 1_000_000_000);
    let period = match cadence {
        Cadence::Second => 1_000_000_000,
        Cadence::Minute => 60_000_000_000,
    };
    let elapsed = match cadence {
        Cadence::Second => nanos,
        Cadence::Minute => u64::from(snapshot.second()) * 1_000_000_000 + nanos,
    };
    Duration::from_nanos(period - elapsed)
}
