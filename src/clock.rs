//! Wall-clock timestamps, in the one format rhost publishes.
//!
//! An audit entry and a session record both say *when*, and both say it the same
//! way: RFC 3339 in UTC. The crate has no date library, and pulling one in for a
//! single format would be a dependency for arithmetic that is a dozen lines.

use std::time::{SystemTime, UNIX_EPOCH};

/// Now, as `YYYY-MM-DDTHH:MM:SSZ`.
pub(crate) fn now_rfc3339() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    rfc3339(seconds)
}

/// Whole seconds since the Unix epoch as UTC RFC 3339, second precision.
pub(crate) fn rfc3339(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Days since 1970-01-01 to a calendar date in the proleptic Gregorian calendar.
///
/// Howard Hinnant's `civil_from_days`: the era arithmetic is what keeps leap
/// years and century rules right without a lookup table.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_position + 2) / 5 + 1) as u32;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_match_the_calendar_at_the_awkward_edges() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(rfc3339(1_079_308_799), "2004-03-14T23:59:59Z");
        assert!(
            now_rfc3339().ends_with('Z') && now_rfc3339().len() == 20,
            "{}",
            now_rfc3339()
        );
    }
}
