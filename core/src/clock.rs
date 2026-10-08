//! The local clock's distance from UTC, for the windows that draw a day.
//!
//! The daemon never needs it: the sun is followed in UTC. The panel and the
//! dashboard do, to put "now" and the day's milestones on a local axis, and
//! both used to ask once, at startup, so a window left open across a DST
//! change drew the whole day an hour off until it was reopened. They now ask
//! again after every whole and half hour of UTC, the only moments a local
//! clock moves: of the 516 changes in the 2026–2027 tzdata, 502 fall on the
//! hour and the other 14 on the half hour (Newfoundland, Adelaide, Broken
//! Hill, Lord Howe).
//!
//! Asking is the window's job, since it means running `date +%z`. Reading the
//! answer and deciding when to ask again live here, so both windows do it the
//! same way.

/// Half an hour, in seconds: the grid every DST change falls on.
const HALF_HOUR: i64 = 30 * 60;

/// Reads a UTC offset as `date +%z` prints it (`+0300`, `-0500`, `+0545`)
/// into seconds east of UTC. [`None`] for anything else, rather than a guess.
pub fn parse_utc_offset(text: &str) -> Option<i32> {
    let text = text.trim();
    let sign = match text.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits = &text[1..];
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = digits[..2].parse().ok()?;
    let minutes: i32 = digits[2..].parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

/// The first whole or half hour of UTC after `unix` (seconds since the
/// epoch): the next moment a local clock can have moved.
pub fn next_half_hour(unix: i64) -> i64 {
    (unix.div_euclid(HALF_HOUR) + 1) * HALF_HOUR
}

/// The local clock's offset from UTC as last read, and when to read it again.
#[derive(Debug, Clone, Copy)]
pub struct LocalOffset {
    /// Seconds east of UTC.
    pub secs: i32,
    /// When the reading may have gone stale: the next whole or half hour
    /// after it was taken.
    stale_at: i64,
}

impl LocalOffset {
    /// A first reading, taken at `now` (seconds since the epoch). One that
    /// failed counts as UTC, which is what the windows always fell back to.
    pub fn new(now: i64, reading: Option<i32>) -> Self {
        Self {
            secs: reading.unwrap_or(0),
            stale_at: next_half_hour(now),
        }
    }

    /// Reads again through `read` once `now` has crossed a whole or half
    /// hour, and otherwise does nothing, so a window can call this on every
    /// poll and still run `date` only twice an hour. A failed reading keeps
    /// the last good one and waits for the next half hour instead of
    /// retrying on every poll.
    pub fn refresh(&mut self, now: i64, read: impl FnOnce() -> Option<i32>) {
        if now < self.stale_at {
            return;
        }
        if let Some(secs) = read() {
            self.secs = secs;
        }
        self.stale_at = next_half_hour(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The EU's clocks go back at 01:00 UTC on 2026-10-25 (Tallinn: EEST at
    /// +3 becomes EET at +2), which is where these timestamps sit.
    const BEFORE: i64 = 1_792_889_999; // 00:59:59 UTC
    const CHANGE: i64 = 1_792_890_000; // 01:00:00 UTC

    #[test]
    fn reads_offsets_east_and_west() {
        assert_eq!(parse_utc_offset("+0300"), Some(3 * 3600));
        assert_eq!(parse_utc_offset("-0500"), Some(-5 * 3600));
        // Nepal and Newfoundland: the minutes count too.
        assert_eq!(parse_utc_offset("+0545"), Some(5 * 3600 + 45 * 60));
        assert_eq!(parse_utc_offset("-0330"), Some(-(3 * 3600 + 30 * 60)));
        assert_eq!(parse_utc_offset("+0000"), Some(0));
        // `date` ends its line with a newline.
        assert_eq!(parse_utc_offset("+0300\n"), Some(3 * 3600));
    }

    #[test]
    fn anything_else_is_none_not_a_guess() {
        for text in [
            "",
            "0300",
            "+03",
            "+03:00",
            "+03a0",
            "+030000",
            "ü0300",
            "+٠٣٠٠",
        ] {
            assert_eq!(parse_utc_offset(text), None, "{text}");
        }
    }

    #[test]
    fn the_next_check_is_the_next_whole_or_half_hour() {
        assert_eq!(next_half_hour(BEFORE), CHANGE);
        // On a boundary itself, the next one is half an hour on.
        assert_eq!(next_half_hour(CHANGE), CHANGE + 1800);
        assert_eq!(next_half_hour(CHANGE + 1799), CHANGE + 1800);
        assert_eq!(next_half_hour(0), 1800);
    }

    #[test]
    fn a_window_left_open_across_the_change_reads_it_within_a_poll() {
        let mut offset = LocalOffset::new(BEFORE - 600, Some(3 * 3600));
        // A poll before the change asks nothing.
        offset.refresh(BEFORE, || panic!("asked before the half hour"));
        assert_eq!(offset.secs, 3 * 3600);
        // The first poll after it asks, and hears the new offset.
        offset.refresh(CHANGE, || Some(2 * 3600));
        assert_eq!(offset.secs, 2 * 3600);
        // Then nothing again until the next half hour.
        offset.refresh(CHANGE + 600, || panic!("asked twice in one half hour"));
        offset.refresh(CHANGE + 1800, || Some(2 * 3600));
        assert_eq!(offset.secs, 2 * 3600);
    }

    #[test]
    fn a_failed_reading_keeps_the_last_good_one() {
        let mut offset = LocalOffset::new(BEFORE, Some(3 * 3600));
        offset.refresh(CHANGE, || None);
        assert_eq!(offset.secs, 3 * 3600);
        // And it waits for the next half hour rather than retrying at once.
        offset.refresh(CHANGE + 1, || panic!("retried before the next half hour"));
    }

    #[test]
    fn a_first_reading_that_fails_counts_as_utc() {
        assert_eq!(LocalOffset::new(BEFORE, None).secs, 0);
    }
}
