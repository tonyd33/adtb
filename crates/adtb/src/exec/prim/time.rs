use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::{AmbiguousOffset, TimeZone};

use super::super::Fault;

pub const MICROS_PER_DAY: i64 = 86_400_000_000;

/// The first and last days of `Date`: the days some microsecond of which is an `Instant`.
pub const DAY_MIN: i64 = i64::MIN.div_euclid(MICROS_PER_DAY);
pub const DAY_MAX: i64 = i64::MAX.div_euclid(MICROS_PER_DAY);

/// Microseconds in 400 Gregorian years, after which the calendar and its weekdays repeat.
const CYCLE: i64 = 146_097 * MICROS_PER_DAY;

/// Returns the days since 1970-01-01 of the proleptic Gregorian date `y-m-d`.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Returns the year, month and day of the date `z` days after 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Returns day `date` with `months` and then `days` added, or `None` when the month arithmetic
/// reaches a day its month lacks and `reject`.
fn add_to_date(date: i64, months: i32, days: i32, reject: bool) -> Result<Option<i64>, Fault> {
    let (y, m, d) = civil_from_days(date);
    let total = y * 12 + i64::from(m) - 1 + i64::from(months);
    let (y, m) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
    let last = days_in_month(y, m);
    let d = if d > last {
        if reject {
            return Ok(None);
        }
        last
    } else {
        d
    };
    let out = days_from_civil(y, m, d) + i64::from(days);
    if !(DAY_MIN..=DAY_MAX).contains(&out) {
        return Err(Fault::ArithmeticOverflow);
    }
    Ok(Some(out))
}

pub fn add_period_date(
    date: i32,
    months: i32,
    days: i32,
    reject: bool,
) -> Result<Option<i32>, Fault> {
    Ok(add_to_date(i64::from(date), months, days, reject)?
        .map(|d| i32::try_from(d).expect("a date's day fits an I32")))
}

pub fn add_period_datetime(
    local: i64,
    months: i32,
    days: i32,
    reject: bool,
) -> Result<Option<i64>, Fault> {
    let (date, tod) = (
        local.div_euclid(MICROS_PER_DAY),
        local.rem_euclid(MICROS_PER_DAY),
    );
    let Some(d) = add_to_date(date, months, days, reject)? else {
        return Ok(None);
    };
    at_day(d, tod).map(Some).ok_or(Fault::ArithmeticOverflow)
}

/// Returns the microseconds of time of day `tod` on day `days`, or `None` outside an `I64`.
pub fn at_day(days: i64, tod: i64) -> Option<i64> {
    i64::try_from(i128::from(days) * i128::from(MICROS_PER_DAY) + i128::from(tod)).ok()
}

/// Returns the zone `name` spells exactly.
pub fn zone(name: &str) -> Result<TimeZone, Fault> {
    TimeZone::get(name)
        .ok()
        .filter(|tz| tz.iana_name() == Some(name))
        .ok_or(Fault::UnknownZone)
}

/// Returns `t` moved by whole 400-year cycles into the range the zone rules are computed over.
fn into_rules(t: i64) -> i64 {
    let lo = Timestamp::MIN.as_microsecond() + 2 * MICROS_PER_DAY;
    let hi = Timestamp::MAX.as_microsecond() - 2 * MICROS_PER_DAY;
    let k = if t > hi {
        -((t - hi) / CYCLE + 1)
    } else if t < lo {
        (lo - t) / CYCLE + 1
    } else {
        0
    };
    t + k * CYCLE
}

fn civil(local: i64) -> DateTime {
    let (y, m, d) = civil_from_days(local.div_euclid(MICROS_PER_DAY));
    let tod = local.rem_euclid(MICROS_PER_DAY);
    let s = tod / 1_000_000;
    DateTime::new(
        y as i16,
        m as i8,
        d as i8,
        (s / 3600) as i8,
        (s / 60 % 60) as i8,
        (s % 60) as i8,
        (tod % 1_000_000 * 1000) as i32,
    )
    .expect("a civil time inside the rules' range")
}

/// Returns the offset in microseconds of `zone` at instant `i`.
fn offset_at(zone: &TimeZone, i: i64) -> i64 {
    let t = into_rules(i);
    let ts = Timestamp::from_microsecond(t).expect("an instant inside the rules' range");
    i64::from(zone.to_offset(ts).seconds()) * 1_000_000
}

/// How a civil time resolves in a zone: the offsets before and after it, equal unless it lies in
/// a gap or an overlap.
struct Offsets {
    before: i64,
    after: i64,
    gap: bool,
}

fn offsets(zone: &TimeZone, local: i64) -> Offsets {
    let t = into_rules(local);
    let us = |o: jiff::tz::Offset| i64::from(o.seconds()) * 1_000_000;
    match zone.to_ambiguous_timestamp(civil(t)).offset() {
        AmbiguousOffset::Unambiguous { offset } => Offsets {
            before: us(offset),
            after: us(offset),
            gap: false,
        },
        AmbiguousOffset::Gap { before, after } => Offsets {
            before: us(before),
            after: us(after),
            gap: true,
        },
        AmbiguousOffset::Fold { before, after } => Offsets {
            before: us(before),
            after: us(after),
            gap: false,
        },
    }
}

/// Returns the civil time of instant `i` in `zone`.
pub fn in_zone(i: i64, zone: &TimeZone) -> Result<i64, Fault> {
    i.checked_add(offset_at(zone, i))
        .ok_or(Fault::ArithmeticOverflow)
}

/// Returns civil time `local` in `zone`, moved forward past a gap, or `None` in a gap when
/// `reject`.
pub fn resolve(local: i64, zone: &TimeZone, reject: bool) -> Result<Option<i64>, Fault> {
    let o = offsets(zone, local);
    if !o.gap {
        to_instant(local, zone)?;
        return Ok(Some(local));
    }
    if reject {
        return Ok(None);
    }
    let moved = local
        .checked_add(o.after - o.before)
        .ok_or(Fault::NoSuchDate)?;
    to_instant(moved, zone)?;
    Ok(Some(moved))
}

/// Returns the instant civil time `local` names in `zone`: the earlier of an overlap, and for a
/// gap the instant of the time moved forward past it.
pub fn to_instant(local: i64, zone: &TimeZone) -> Result<i64, Fault> {
    local
        .checked_sub(offsets(zone, local).before)
        .ok_or(Fault::ArithmeticOverflow)
}

fn year_text(y: i64) -> String {
    if (0..=9999).contains(&y) {
        format!("{y:04}")
    } else {
        format!("{}{:05}", if y < 0 { '-' } else { '+' }, y.unsigned_abs())
    }
}

pub fn date_text(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{}-{m:02}-{d:02}", year_text(y))
}

pub fn time_text(tod: i64) -> String {
    let s = tod / 1_000_000;
    format!(
        "{:02}:{:02}:{:02}.{:06}",
        s / 3600,
        s / 60 % 60,
        s % 60,
        tod % 1_000_000
    )
}

pub fn datetime_text(local: i64) -> String {
    format!(
        "{}T{}",
        date_text(local.div_euclid(MICROS_PER_DAY)),
        time_text(local.rem_euclid(MICROS_PER_DAY))
    )
}

/// The units of a printed `Duration`, largest first.
pub const UNITS: [(&str, i64); 6] = [
    ("d", MICROS_PER_DAY),
    ("h", 3_600_000_000),
    ("min", 60_000_000),
    ("s", 1_000_000),
    ("ms", 1_000),
    ("us", 1),
];

pub fn duration_text(d: i64) -> String {
    if d == 0 {
        return "0s".into();
    }
    let m = d.unsigned_abs();
    let (unit, per) = UNITS
        .iter()
        .find(|(_, per)| m.is_multiple_of(per.unsigned_abs()))
        .expect("a microsecond divides every duration");
    let sign = if d < 0 { "-" } else { "" };
    format!("{sign}{}{unit}", m / per.unsigned_abs())
}
