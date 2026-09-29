//! Provider timestamps: reading an ISO-8601 stamp, and refusing a confirmed
//! transaction that arrives without a time.

/// A history entry's time, in the provider's own unit: `None` only while the
/// chain has not given the transaction one — it is not yet in a block.
///
/// A confirmed transaction always has a time, so one that arrives without it
/// was read wrongly, and the fetch fails naming it rather than dating it 1970.
pub(crate) fn history_time(
    confirmed: bool,
    time: Option<u64>,
    txid: &str,
) -> Result<Option<u64>, String> {
    match time.filter(|t| *t > 0) {
        None if !confirmed => Ok(None),
        time => confirmed_history_time(time, txid).map(Some),
    }
}

/// The time of a transaction from a source that lists only confirmed ones.
pub(crate) fn confirmed_history_time(time: Option<u64>, txid: &str) -> Result<u64, String> {
    time.filter(|t| *t > 0)
        .ok_or_else(|| format!("history: confirmed transaction {txid} has no time"))
}

/// Parse an RFC 3339 / ISO-8601 timestamp to Unix seconds, or `None` when the
/// string is not one. The caller decides what an unreadable stamp becomes.
///
/// Hand-rolled to keep a date-time crate out of the tree, and hand-rolled
/// carefully — the parser this replaces got two things wrong that a provider's
/// response could reach:
///
/// - it indexed the string by byte offset without checking char boundaries, so
///   any non-ASCII character in a response of nineteen bytes or more panicked
///   inside core rather than failing to parse;
/// - it read every stamp as UTC, including the `+00:00` form its own comment
///   claimed to support, so an offset stamp landed hours away from its instant.
///
/// It also answered `0.0` for anything it could not read, which is a real date
/// and not a refusal.
pub(crate) fn parse_iso8601_timestamp(s: &str) -> Option<f64> {
    let s = s.trim();
    // Every byte index below is sound exactly because of this check: a
    // timestamp is ASCII by definition, and anything else is not one.
    if !s.is_ascii() {
        return None;
    }
    let bytes = s.as_bytes();
    // `YYYY-MM-DDTHH:MM:SS` is the shortest form read. Fractional seconds and
    // a zone offset may follow, and are handled by `zone_offset_seconds`.
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }

    let field = |range: std::ops::Range<usize>| -> Option<i64> {
        let text = s.get(range)?;
        // `parse` would take a sign, and `+123-01-01…` is not a date.
        text.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| text.parse().ok())
            .flatten()
    };
    let year = field(0..4)?;
    let month = field(5..7)?;
    let day = field(8..10)?;
    let hour = field(11..13)?;
    let minute = field(14..16)?;
    let second = field(17..19)?;
    // Ranges, so a malformed field cannot roll the date somewhere plausible.
    // Second 60 stays in: a leap second is a real reading.
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }

    let wall = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    // The fields above are a wall clock in the stamp's own zone; the offset is
    // how far that zone runs ahead of UTC.
    Some((wall - zone_offset_seconds(&s[19..])?) as f64)
}

/// How far the stamp's zone runs ahead of UTC, in seconds.
///
/// Accepts `Z`, an empty suffix (both UTC) and `±HH:MM` / `±HHMM` / `±HH`,
/// each optionally preceded by fractional seconds. Anything else means the
/// string was not the timestamp it looked like, so it is `None` rather than a
/// silent zero.
fn zone_offset_seconds(suffix: &str) -> Option<i64> {
    // Fractional seconds sit between the seconds field and the zone. Their
    // precision is below what a history row renders, so they are skipped.
    let suffix = match suffix.strip_prefix('.') {
        Some(rest) => rest.trim_start_matches(|c: char| c.is_ascii_digit()),
        None => suffix,
    };
    if suffix.is_empty() || suffix.eq_ignore_ascii_case("Z") {
        return Some(0);
    }
    let (sign, body) = match suffix.as_bytes()[0] {
        b'+' => (1, &suffix[1..]),
        b'-' => (-1, &suffix[1..]),
        _ => return None,
    };
    let digits: String = body.chars().filter(|c| *c != ':').collect();
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (hours, minutes) = match digits.len() {
        2 => (digits.parse::<i64>().ok()?, 0),
        4 => (
            digits[..2].parse::<i64>().ok()?,
            digits[2..].parse::<i64>().ok()?,
        ),
        _ => return None,
    };
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// A provider's timestamp is a string from the network, so the parser has to
/// treat it as one: it may be malformed, it may carry a zone, and it may not
/// be ASCII at all.
#[cfg(test)]
mod iso8601_tests {
    use super::parse_iso8601_timestamp;

    /// `2023-11-14T22:13:20Z` is Unix 1700000000, and every spelling of that
    /// instant has to reach the same number. The parser this replaces read
    /// the wall clock and dropped the offset, so the last two of these landed
    /// eight and five hours away from the first.
    #[test]
    fn one_instant_spelled_five_ways_is_one_number() {
        for spelling in [
            "2023-11-14T22:13:20Z",
            "2023-11-14T22:13:20",
            "2023-11-14t22:13:20z",
            "2023-11-14 22:13:20",
            "2023-11-14T22:13:20.123456Z",
            "2023-11-15T06:13:20+08:00",
            "2023-11-15T06:13:20+0800",
            "2023-11-15T06:13:20+08",
            "2023-11-14T17:13:20-05:00",
            "  2023-11-14T22:13:20Z  ",
        ] {
            assert_eq!(
                parse_iso8601_timestamp(spelling),
                Some(1_700_000_000.0),
                "{spelling}"
            );
        }
    }

    /// Byte-indexing a `&str` at 4, 7, 10, 13 and 16 panics when one of those
    /// offsets falls inside a multi-byte character. These are all at least
    /// nineteen bytes, and each must be refused rather than abort core.
    #[test]
    fn a_non_ascii_response_is_refused_and_does_not_panic() {
        for hostile in [
            "２０２３-11-14T22:13:20Z",
            "2023-11-14T22:13:20Z…………",
            "日本語日本語日本語日本語日本語日本語",
            "2023-11-14T22:13:2\u{0660}Z",
        ] {
            assert_eq!(parse_iso8601_timestamp(hostile), None, "{hostile}");
        }
    }

    /// Refusal, not the epoch. `0.0` is 1 January 1970 — a real date that a
    /// history row renders as one and sorts by.
    #[test]
    fn what_is_not_a_timestamp_is_none_rather_than_1970() {
        for malformed in [
            "",
            "2023-11-14",
            "not a timestamp at all",
            "2023/11/14T22:13:20Z",
            "2023-11-14X22:13:20Z",
            "2023-13-14T22:13:20Z", // month 13
            "2023-11-32T22:13:20Z", // day 32
            "2023-11-14T24:13:20Z", // hour 24
            "2023-11-14T22:60:20Z", // minute 60
            "2023-11-14T22:13:61Z", // second 61
            "20a3-11-14T22:13:20Z",
            "+023-11-14T22:13:20Z",
            "2023-11-14T22:13:20+24:00", // offset out of range
            "2023-11-14T22:13:20+8",     // offset too short to read
            "2023-11-14T22:13:20 UTC",
        ] {
            assert_eq!(parse_iso8601_timestamp(malformed), None, "{malformed}");
        }
    }

    /// A leap second is a real reading at :60, and the epoch itself is a real
    /// timestamp rather than a failure value.
    #[test]
    fn leap_seconds_and_the_epoch_itself_parse() {
        assert_eq!(
            parse_iso8601_timestamp("2016-12-31T23:59:60Z"),
            Some(1_483_228_800.0)
        );
        assert_eq!(
            parse_iso8601_timestamp("1970-01-01T00:00:00Z"),
            Some(0.0),
            "the epoch parses; it is no longer also the error value"
        );
    }
}
