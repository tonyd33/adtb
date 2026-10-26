//! Zone rules: civil times of instants, civil times in gaps and overlaps, and arithmetic on
//! zoned date-times.

use adtb_reference::{Abort, Fault, Value, days_from_civil};
use adtb_testkit::fixture::*;

const HOUR: i64 = 3_600_000_000;
const DAY: i64 = 24 * HOUR;

fn fixture() -> Fixture {
    Fixture::new(
        "type Note = { id: Key notes, body: Text }\nrelation notes : Note key id generate sequence",
    )
}

fn civil(y: i64, m: u32, d: u32, h: i64, min: i64) -> i64 {
    days_from_civil(y, m, d) * DAY + h * HOUR + min * 60_000_000
}

fn instant(name: &str, v: i64) -> [(&str, &'static str, Value); 1] {
    [(name, "Instant", Value::Instant(v))]
}

fn datetime(name: &str, v: i64) -> [(&str, &'static str, Value); 1] {
    [(name, "DateTime", Value::DateTime(v))]
}

fn zoned(c: i64, zone: &str) -> Value {
    Value::Zoned(c, zone.to_string())
}

#[test]
fn in_zone_gives_the_civil_time() {
    let f = fixture();
    let t = civil(2026, 7, 1, 12, 0);
    assert_eq!(
        Ok(zoned(civil(2026, 7, 1, 8, 0), "America/New_York")),
        f.eval_with(r#"in_zone(t, "America/New_York")"#, &instant("t", t))
    );
    let t = civil(2026, 1, 1, 12, 0);
    assert_eq!(
        Ok(zoned(civil(2026, 1, 1, 7, 0), "America/New_York")),
        f.eval_with(r#"in_zone(t, "America/New_York")"#, &instant("t", t))
    );
    assert_eq!(
        Ok(zoned(civil(2026, 1, 1, 17, 45), "Asia/Kathmandu")),
        f.eval_with(r#"in_zone(t, "Asia/Kathmandu")"#, &instant("t", t))
    );
}

#[test]
fn an_unknown_zone_faults() {
    let f = fixture();
    let t = instant("t", 0);
    let unknown = Err(Abort::Fault(Fault::UnknownZone));
    assert_eq!(unknown, f.eval_with(r#"in_zone(t, "Nowhere/Special")"#, &t));
    assert_eq!(
        unknown,
        f.eval_with(r#"in_zone(t, "europe/lisbon")"#, &t),
        "a zone name is spelled as the zone rules spell it"
    );
    let dt = datetime("dt", 0);
    assert_eq!(unknown, f.eval_with(r#"zoned(dt, "", Clamp)"#, &dt));
}

#[test]
fn zoned_resolves_gaps_and_overlaps() {
    let f = fixture();
    let gap = datetime("dt", civil(2026, 3, 8, 2, 30));
    assert_eq!(
        Ok(Value::none()),
        f.eval_with(r#"zoned(dt, "America/New_York", Reject)"#, &gap)
    );
    assert_eq!(
        Ok(zoned(civil(2026, 3, 8, 3, 30), "America/New_York")),
        f.eval_with(r#"zoned(dt, "America/New_York", Clamp)"#, &gap),
        "Clamp moves forward by the length of the gap"
    );
    assert_eq!(
        Ok(Value::Instant(civil(2026, 3, 8, 7, 30))),
        f.eval_with(r#"to_instant(zoned(dt, "America/New_York", Clamp))"#, &gap)
    );
    let overlap = datetime("dt", civil(2026, 11, 1, 1, 30));
    assert_eq!(
        Ok(Value::Instant(civil(2026, 11, 1, 5, 30))),
        f.eval_with(
            r#"to_instant(zoned(dt, "America/New_York", Clamp))"#,
            &overlap
        ),
        "an overlap names the earlier instant"
    );
    assert_eq!(
        Ok(Value::some(zoned(
            civil(2026, 11, 1, 1, 30),
            "America/New_York"
        ))),
        f.eval_with(r#"zoned(dt, "America/New_York", Reject)"#, &overlap)
    );
}

#[test]
fn to_instant_inverts_in_zone() {
    let f = fixture();
    for t in [
        0,
        civil(2026, 3, 8, 6, 59),
        civil(2026, 11, 1, 5, 0),
        -civil(3000, 1, 1, 0, 0),
    ] {
        assert_eq!(
            Ok(Value::Instant(t)),
            f.eval_with(
                r#"to_instant(in_zone(t, "America/New_York"))"#,
                &instant("t", t)
            )
        );
    }
}

#[test]
fn the_later_instant_of_an_overlap_reads_back_as_the_earlier() {
    let f = fixture();
    let later = instant("t", civil(2026, 11, 1, 6, 0));
    assert_eq!(
        Ok(Value::Instant(civil(2026, 11, 1, 5, 0))),
        f.eval_with(r#"to_instant(in_zone(t, "America/New_York"))"#, &later)
    );
}

#[test]
fn zone_rules_hold_far_from_the_present() {
    let f = fixture();
    let summer = civil(30_000, 7, 1, 12, 0);
    assert_eq!(
        Ok(zoned(civil(30_000, 7, 1, 8, 0), "America/New_York")),
        f.eval_with(r#"in_zone(t, "America/New_York")"#, &instant("t", summer))
    );
    let winter = civil(30_000, 1, 1, 12, 0);
    assert_eq!(
        Ok(zoned(civil(30_000, 1, 1, 7, 0), "America/New_York")),
        f.eval_with(r#"in_zone(t, "America/New_York")"#, &instant("t", winter))
    );
    let ancient = civil(-30_000, 1, 1, 12, 0);
    assert_eq!(
        Ok(zoned(ancient - 17_762 * 1_000_000, "America/New_York")),
        f.eval_with(r#"in_zone(t, "America/New_York")"#, &instant("t", ancient)),
        "local mean time before the first transition"
    );
}

#[test]
fn a_duration_moves_the_instant_then_shows_it_in_the_zone() {
    let f = fixture();
    let before = instant("t", civil(2026, 3, 8, 6, 30));
    assert_eq!(
        Ok(zoned(civil(2026, 3, 8, 3, 30), "America/New_York")),
        f.eval_with(r#"in_zone(t, "America/New_York") + hours(1)"#, &before),
        "1:30 EST plus an hour is 3:30 EDT"
    );
    assert_eq!(
        Ok(zoned(civil(2026, 3, 8, 0, 30), "America/New_York")),
        f.eval_with(r#"in_zone(t, "America/New_York") - hours(1)"#, &before)
    );
}

#[test]
fn add_period_keeps_the_wall_time() {
    let f = fixture();
    let t = instant("t", civil(2026, 3, 7, 7, 30));
    assert_eq!(
        Ok(zoned(civil(2026, 3, 8, 3, 30), "America/New_York")),
        f.eval_with(
            r#"add_period(in_zone(t, "America/New_York"), days(1), Clamp)"#,
            &t
        ),
        "2:30 does not exist on the next day; Clamp moves it forward"
    );
    assert_eq!(
        Ok(Value::none()),
        f.eval_with(
            r#"add_period(in_zone(t, "America/New_York"), days(1), Reject)"#,
            &t
        )
    );
    let jan31 = instant("t", civil(2026, 1, 31, 17, 0));
    assert_eq!(
        Ok(zoned(civil(2026, 2, 28, 12, 0), "America/New_York")),
        f.eval_with(
            r#"add_period(in_zone(t, "America/New_York"), months(1), Clamp)"#,
            &jan31
        )
    );
}

#[test]
fn a_zoned_value_prints_its_civil_time_and_zone() {
    let f = fixture();
    let t = instant("t", civil(2026, 10, 25, 8, 30));
    assert_eq!(
        Ok(text("2026-10-25T09:30:00.000000[Europe/Paris]")),
        f.eval_with(r#"text(in_zone(t, "Europe/Paris"))"#, &t)
    );
}
