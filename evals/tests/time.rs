//! The window parser refuses what would silently shift a window, and the formatter echoes it back.

use cort_evals::time::{format_utc, parse_since};

#[test]
fn a_window_without_an_offset_is_refused_rather_than_guessed() {
    // The hand-run that produced a full-zero funnel passed a local wall-clock time and got it read
    // as UTC, placing the cutoff eight hours in the future. Both readings are defensible, which is
    // exactly why neither may be assumed.
    let err = parse_since("2026-09-02 09:24").unwrap_err();
    assert!(err.contains("explicit UTC offset"), "{err}");
    let err = parse_since("2026-09-02T09:24:00").unwrap_err();
    assert!(err.contains("explicit UTC offset"), "{err}");

    // The same instant, written two ways, is one number.
    let taipei = parse_since("2026-09-02T09:24:00+08:00").unwrap();
    let utc = parse_since("2026-09-02T01:24:00Z").unwrap();
    assert_eq!(taipei, utc);
    assert_eq!(format_utc(taipei), "2026-09-02T01:24:00Z");
    // And a negative offset moves the other way.
    assert_eq!(parse_since("2026-09-01T21:24:00-04:00").unwrap(), utc);
}

/// Codex's ninth finding: refusing offset-less input fixed one silent window error, not all of them.
#[test]
fn an_impossible_instant_is_refused_too() {
    assert!(
        parse_since("2026-02-31T00:00:00Z").is_err(),
        "2026-02-31 is not a date"
    );
    assert!(
        parse_since("2026-09-02T00:00:00+99:00").is_err(),
        "no zone is +99"
    );
    assert!(
        parse_since("2026-09-02T00:00:00.5xZ").is_err(),
        "junk is not a fraction"
    );
    // The real leap day is a date.
    assert!(parse_since("2024-02-29T00:00:00Z").is_ok());
    assert!(parse_since("2026-02-28T23:59:59.250Z").is_ok());
}
