//! Underruns the device reports, counted where they happen and logged in sums.
//!
//! The device's error callback is called once per underrun, on the device's
//! own worker thread, and under load that is several times a second. A log
//! line each time is a log that drowns, and a log written from the thread
//! that is already late makes it later. So the callback only counts (one
//! atomic add), and the thread that owns the stream reads the count at most
//! once a second and writes one line with the number in it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How often a run of underruns may be logged, at most.
pub const REPORT_EVERY: Duration = Duration::from_secs(1);

/// Underruns since the stream opened, and since they were last logged.
#[derive(Debug, Default)]
pub(crate) struct Underruns {
    total: AtomicU64,
    unreported: AtomicU64,
    /// When the last line was written. Only the reporting thread takes it.
    reported_at: Mutex<Option<Instant>>,
}

impl Underruns {
    /// One underrun. Called from the device's error callback: an atomic add,
    /// nothing else.
    pub fn note(&self) {
        self.total.fetch_add(1, Ordering::Relaxed);
        self.unreported.fetch_add(1, Ordering::Relaxed);
    }

    /// Every underrun since the stream opened.
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// The underruns to log now, if a line is due: some have happened since
    /// the last line, and the last line is at least `REPORT_EVERY` old. Those
    /// not yet due stay counted for the next line, so none is lost.
    pub fn due(&self, now: Instant) -> Option<u64> {
        let Ok(mut reported_at) = self.reported_at.lock() else { return None };
        if reported_at.is_some_and(|at| now.saturating_duration_since(at) < REPORT_EVERY) {
            return None;
        }
        let count = self.unreported.swap(0, Ordering::Relaxed);
        if count == 0 {
            return None;
        }
        *reported_at = Some(now);
        Some(count)
    }

    /// Logs one line if one is due. Called from the thread that owns the
    /// stream, never from the audio callback.
    pub fn report(&self, now: Instant) {
        if let Some(count) = self.due(now) {
            tracing::warn!(count, total = self.total(), "audio buffer underruns or overruns since the last report");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_is_one_line_with_its_count() {
        let underruns = Underruns::default();
        let start = Instant::now();
        for _ in 0..7 {
            underruns.note();
        }
        assert_eq!(underruns.due(start), Some(7));
        assert_eq!(underruns.due(start), None, "nothing new, nothing to say");
        assert_eq!(underruns.total(), 7);
    }

    #[test]
    fn at_most_one_line_a_second_and_nothing_is_lost_in_between() {
        let underruns = Underruns::default();
        let start = Instant::now();
        underruns.note();
        assert_eq!(underruns.due(start), Some(1));
        // Several more within the second: held, not logged and not dropped.
        let mut lines = 0;
        for ms in (0..1000).step_by(50) {
            underruns.note();
            if underruns.due(start + Duration::from_millis(ms)).is_some() {
                lines += 1;
            }
        }
        assert_eq!(lines, 0, "a second line inside the second");
        assert_eq!(underruns.due(start + REPORT_EVERY), Some(20));
        assert_eq!(underruns.total(), 21);
    }

    #[test]
    fn a_quiet_stream_never_logs() {
        let underruns = Underruns::default();
        let start = Instant::now();
        for s in 0..5 {
            assert_eq!(underruns.due(start + Duration::from_secs(s)), None);
        }
        assert_eq!(underruns.total(), 0);
    }
}
