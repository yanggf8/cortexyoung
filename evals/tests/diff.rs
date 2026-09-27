//! `run-diff` properties: a diff may subtract only what the instrument measured the same way,
//! and must say when it did not.

use cort_evals::diff;
use serde_json::{json, Value};

fn report(since_ms: i64, searches: i64, machine: &str) -> Value {
    json!({
        "window": {"since_ms": since_ms, "since_utc": "x", "until_ms": since_ms + 86_400_000, "until_utc": "y"},
        "searches": searches,
        "injections": 0,
        "machine": {"id": machine, "source": "fixture"},
        "demand_context": {"captured_like": searches},
        "by_project": {
            "-home-u-repo": {"searches": searches, "injections": 0},
            "-home-u-gone": {"searches": 1, "injections": 0},
        },
    })
}

/// The 2026-09-27 calibre break: parser widened, hook muted, two indexes bootstrapped. A diff
/// whose windows straddle it must print the note, because the numbers on either side are
/// different numbers whatever the subtraction says.
#[test]
fn a_diff_across_the_2026_09_27_break_names_it() {
    let old = report(1_790_092_800_000, 10, "m"); // 2026-09-23 +08
    let new = report(1_790_524_800_000, 25, "m"); // 2026-09-28 +08, a day past the break
    let d = diff::diff(&old, &new).unwrap();
    let breaks = d["calibre_breaks"].as_array().unwrap();
    assert_eq!(breaks.len(), 1, "the one known break: {d}");
    assert!(breaks[0]["why"].as_str().unwrap().contains("eab18be9"));
}

/// Two windows on the same side of every break subtract plainly.
#[test]
fn a_diff_within_one_calibre_has_no_breaks() {
    let old = report(1_790_438_400_000, 10, "m");
    let new = report(1_790_784_000_000, 25, "m");
    let d = diff::diff(&old, &new).unwrap();
    assert_eq!(d["calibre_breaks"].as_array().unwrap().len(), 0);
    assert_eq!(d["scalars"]["searches"]["delta"], json!(15));
    assert_eq!(d["scalars"]["searches"]["old"], json!(10));
}

/// Rows carry no machine of their own; two machines' reports cannot be separated after the fact.
#[test]
fn reports_from_different_machines_are_refused() {
    let err = diff::diff(&report(0, 1, "m1"), &report(1, 2, "m2")).unwrap_err();
    assert!(err.contains("different machines"), "got: {err}");
}

/// A project that exists on only one side is still listed -- absence is a datum.
#[test]
fn a_project_present_on_one_side_is_listed_with_a_null() {
    let mut new = report(1_790_784_000_000, 25, "m");
    new["by_project"]
        .as_object_mut()
        .unwrap()
        .remove("-home-u-gone");
    new["by_project"].as_object_mut().unwrap().insert(
        "-home-u-fresh".to_string(),
        json!({"searches": 3, "injections": 0}),
    );
    let d = diff::diff(&report(1_790_438_400_000, 10, "m"), &new).unwrap();
    assert_eq!(
        d["by_project"]["-home-u-gone"]["searches"]["new"],
        json!(null)
    );
    assert_eq!(d["by_project"]["-home-u-gone"]["searches"]["old"], json!(1));
    assert_eq!(
        d["by_project"]["-home-u-fresh"]["searches"]["old"],
        json!(null)
    );
    assert_eq!(
        d["by_project"]["-home-u-fresh"]["searches"]["new"],
        json!(3)
    );
}

/// A file without a window is not an adopt-mine report; guessing one would invent a calibre.
#[test]
fn a_report_without_a_window_is_refused() {
    let err = diff::diff(
        &json!({"searches": 1, "machine": {"id": "m"}}),
        &report(1, 1, "m"),
    )
    .unwrap_err();
    assert!(err.contains("window.since_ms"), "got: {err}");
}
