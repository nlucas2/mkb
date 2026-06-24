//! Timestamp helpers: the single place that knows mdkb's on-the-wire time format.
//!
//! mdkb represents times as **RFC 3339 in UTC at second precision** (`2026-06-24T02:30:00Z`).
//! That form is fixed-width, so lexical string comparison is also chronological comparison —
//! date-range filters compare stored values as plain strings and only need to parse the *user's*
//! query input. `created` is derived from a block's ULID id; `updated` is stamped on each write.
//!
//! Everything here is total and panic-free: malformed or out-of-range inputs yield `None`, never a
//! crash, so reading a block whose `updated:` is absent or odd is always safe.

use time::format_description::well_known::Rfc3339;
use time::{Date, Month, OffsetDateTime, Time, UtcOffset};

/// The current UTC time as an RFC 3339 string at second precision. Empty string only in the
/// impossible case that the platform clock can't be formatted.
pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .ok()
        .and_then(|dt| dt.format(&Rfc3339).ok())
        .unwrap_or_default()
}

/// Convert Unix milliseconds (e.g. a ULID's embedded timestamp) to an RFC 3339 UTC string at
/// second precision, or `None` if the value is out of the representable range.
pub fn unix_ms_to_rfc3339(ms: u64) -> Option<String> {
    let secs = (ms / 1000) as i64;
    OffsetDateTime::from_unix_timestamp(secs)
        .ok()?
        .format(&Rfc3339)
        .ok()
}

/// Normalize a user-supplied date filter into the canonical RFC 3339 UTC second-precision form
/// used for comparison, or `None` if it isn't a recognizable date. Accepts either a full RFC 3339
/// timestamp (`2026-06-24T02:30:00Z`) or a bare calendar date (`2026-06-24`, taken as midnight
/// UTC). Because stored times share this exact format, the returned string can be compared
/// lexically against them.
pub fn parse_query_date(input: &str) -> Option<String> {
    let s = input.trim();
    if let Ok(dt) = OffsetDateTime::parse(s, &Rfc3339) {
        // Convert to UTC before formatting: an offset-bearing input (e.g. `…+05:00`) must become
        // the canonical `…Z` form, or lexical comparison against stored (always-UTC) values would
        // be wrong even though the instant is correct.
        return dt
            .to_offset(UtcOffset::UTC)
            .replace_nanosecond(0)
            .ok()?
            .format(&Rfc3339)
            .ok();
    }
    // Bare `YYYY-MM-DD` → midnight UTC.
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 3 {
        let y: i32 = parts[0].parse().ok()?;
        let m: u8 = parts[1].parse().ok()?;
        let d: u8 = parts[2].parse().ok()?;
        let month = Month::try_from(m).ok()?;
        let date = Date::from_calendar_date(y, month, d).ok()?;
        return OffsetDateTime::new_utc(date, Time::MIDNIGHT)
            .format(&Rfc3339)
            .ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_rfc3339_seconds_utc() {
        let s = now_rfc3339();
        // e.g. 2026-06-24T02:30:00Z — ends in Z, no fractional seconds.
        assert!(s.ends_with('Z'), "got {s}");
        assert!(!s.contains('.'), "should be second precision: {s}");
        // Round-trips back through a parse.
        assert!(OffsetDateTime::parse(&s, &Rfc3339).is_ok());
    }

    #[test]
    fn ulid_millis_decode_to_a_date() {
        // The Unix epoch is unambiguous.
        assert_eq!(unix_ms_to_rfc3339(0).unwrap(), "1970-01-01T00:00:00Z");
        // Sub-second remainder is truncated, not rounded.
        assert_eq!(unix_ms_to_rfc3339(1_999).unwrap(), "1970-01-01T00:00:01Z");
        // Round-trips back to the same whole second through the query parser.
        let s = unix_ms_to_rfc3339(1_782_621_000_000).unwrap();
        assert_eq!(parse_query_date(&s).unwrap(), s);
    }

    #[test]
    fn query_dates_normalize_and_compare_lexically() {
        let bare = parse_query_date("2026-06-24").unwrap();
        assert_eq!(bare, "2026-06-24T00:00:00Z");
        let full = parse_query_date("2026-06-24T02:30:00Z").unwrap();
        assert_eq!(full, "2026-06-24T02:30:00Z");
        // Lexical order is chronological for this fixed format.
        assert!(bare < full);
        assert!(parse_query_date("not-a-date").is_none());
        assert!(parse_query_date("2026-13-40").is_none());
    }

    #[test]
    fn offset_query_dates_normalize_to_utc() {
        // A non-UTC offset must be converted to the canonical `…Z` form so lexical comparison
        // against stored UTC values stays chronological.
        assert_eq!(
            parse_query_date("2026-06-24T02:30:00+05:00").unwrap(),
            "2026-06-23T21:30:00Z"
        );
        assert_eq!(
            parse_query_date("2026-06-24T02:30:00-08:00").unwrap(),
            "2026-06-24T10:30:00Z"
        );
        // Same instant, different spellings, normalize identically.
        assert_eq!(
            parse_query_date("2026-06-24T10:00:00Z").unwrap(),
            parse_query_date("2026-06-24T02:30:00-07:30").unwrap()
        );
    }
}
