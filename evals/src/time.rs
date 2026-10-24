//! UTC instant parsing and formatting shared by the cost lanes. Moved verbatim out of the
//! removed adoption module; behaviour is unchanged.

/// Milliseconds since the epoch for an RFC 3339 timestamp that carries an explicit offset.
///
/// A bare `2026-09-02 09:24` is refused rather than assumed to be UTC or local. Either assumption
/// is silently wrong half the time, and the wrong one produced a full-zero funnel that read exactly
/// like a broken hook.
pub fn parse_since(raw: &str) -> Result<i64, String> {
    parse_instant("--since", raw)
}

fn parse_instant(flag: &str, raw: &str) -> Result<i64, String> {
    let s = raw.trim();
    let refuse = |why: &str| -> String {
        format!(
            "{flag} {raw}: {why}\nit must carry an explicit UTC offset, e.g. \
             2026-09-02T09:24:00+08:00 or 2026-09-02T01:24:00Z -- transcript timestamps are UTC \
             and a local wall-clock time without an offset silently reads as the wrong instant"
        )
    };
    let (civil, offset_min) = if let Some(head) = s.strip_suffix('Z').or(s.strip_suffix('z')) {
        (head.to_string(), 0i64)
    } else {
        // The offset sign has to be looked for after the date, or `2026-09-02` splits on its own
        // dashes and the year becomes an offset.
        let Some(time_at) = s.find(['T', 't']) else {
            return Err(refuse("no time-of-day"));
        };
        let Some(sign_at) = s[time_at..].find(['+', '-']).map(|i| i + time_at) else {
            return Err(refuse("no offset"));
        };
        let (head, off) = s.split_at(sign_at);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let digits = &off[1..];
        let (oh, om) = match digits.split_once(':') {
            Some((h, m)) => (h.to_string(), m.to_string()),
            None if digits.len() == 4 => (digits[..2].to_string(), digits[2..].to_string()),
            None => return Err(refuse("malformed offset")),
        };
        let oh: i64 = oh.parse().map_err(|_| refuse("malformed offset"))?;
        let om: i64 = om.parse().map_err(|_| refuse("malformed offset"))?;
        // No real zone is past +/-14:00. An unchecked offset silently shifts the window, which is
        // the same class of error as the one that made this function refuse bare local times.
        if oh > 14 || om > 59 || (oh == 14 && om > 0) {
            return Err(refuse("offset out of range"));
        }
        (head.to_string(), sign * (oh * 60 + om))
    };
    let ms = civil_to_ms(&civil).ok_or_else(|| refuse("malformed date-time"))?;
    Ok(ms - offset_min * 60_000)
}

/// `YYYY-MM-DDTHH:MM[:SS[.fff]]` read as if it were UTC. The caller applies the offset.
fn civil_to_ms(s: &str) -> Option<i64> {
    let (date, time) = s.split_once(['T', 't'])?;
    let mut d = date.split('-');
    let y: i64 = d.next()?.parse().ok()?;
    let m: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    if d.next().is_some() || !(1..=12).contains(&m) || day < 1 || day > days_in_month(y, m) {
        return None;
    }
    let mut t = time.split(':');
    let hh: i64 = t.next()?.parse().ok()?;
    let mm: i64 = t.next()?.parse().ok()?;
    let (ss, frac) = match t.next() {
        Some(rest) => match rest.split_once('.') {
            Some((sec, f)) => (sec.parse::<i64>().ok()?, millis_of_fraction(f)?),
            None => (rest.parse::<i64>().ok()?, 0),
        },
        None => (0, 0),
    };
    if t.next().is_some() || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    Some((days_from_civil(y, m, day) * 86_400 + hh * 3600 + mm * 60 + ss) * 1000 + frac)
}

/// `2026-02-31` is not a date. Accepting it silently moves the window by a day.
fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// `None` for anything that is not all digits: `.5x` is not a fraction, and reading it as `.500`
/// would be inventing a timestamp the caller never wrote.
fn millis_of_fraction(f: &str) -> Option<i64> {
    if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = f.as_bytes();
    let mut ms = 0i64;
    for i in 0..3 {
        ms = ms * 10 + digits.get(i).map_or(0, |b| (b - b'0') as i64);
    }
    Some(ms)
}

/// Howard Hinnant's days-from-civil. Proleptic Gregorian, no table, no dependency: the alternative
/// was a date crate for one conversion in a dev-only binary.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The UTC instant, for the report to echo back. A window the reader cannot check is a window they
/// have to trust, and trusting it is what went wrong.
pub fn format_utc(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (hh, mm, ss) = (rem / 3_600_000, (rem / 60_000) % 60, (rem / 1000) % 60);
    // Inverse of days_from_civil.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}
