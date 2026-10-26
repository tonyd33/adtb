use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::{AmbiguousOffset, TimeZone};

use crate::eval::{Abort, Fault, R};
use crate::time::{MICROS_PER_DAY, civil_from_days, days_from_civil};

const MICROS_PER_SECOND: i64 = 1_000_000;

/// The length of 400 Gregorian years, after which the calendar and every rule a zone states for
/// all future years repeat.
const CYCLE: i64 = 146_097 * MICROS_PER_DAY;

/// Returns the rules of zone `name`, which must be spelled as the zone rules spell it.
pub fn lookup(name: &str) -> R<TimeZone> {
    match jiff::tz::db().get(name) {
        Ok(tz) if tz.iana_name() == Some(name) => Ok(tz),
        _ => Err(Abort::Fault(Fault::UnknownZone)),
    }
}

pub fn exists(name: &str) -> bool {
    lookup(name).is_ok()
}

/// Returns `micros` moved by whole cycles into the years -9000 to 9000.
fn into_window(micros: i64) -> i64 {
    let lo = days_from_civil(-9000, 1, 1) * MICROS_PER_DAY;
    let hi = days_from_civil(9000, 1, 1) * MICROS_PER_DAY;
    let k = if micros > hi {
        -((micros - hi) / CYCLE + 1)
    } else if micros < lo {
        (lo - micros) / CYCLE + 1
    } else {
        0
    };
    micros + k * CYCLE
}

fn seconds(o: jiff::tz::Offset) -> i64 {
    i64::from(o.seconds()) * MICROS_PER_SECOND
}

/// Returns the offset of zone `tz` at instant `micros`, in microseconds.
fn offset_at(tz: &TimeZone, micros: i64) -> i64 {
    let ts = Timestamp::from_microsecond(into_window(micros))
        .expect("the window is within jiff's range");
    seconds(tz.to_offset(ts))
}

fn jiff_civil(micros: i64) -> DateTime {
    let (days, rest) = (
        micros.div_euclid(MICROS_PER_DAY),
        micros.rem_euclid(MICROS_PER_DAY),
    );
    let (y, m, d) = civil_from_days(days);
    let s = rest / MICROS_PER_SECOND;
    DateTime::new(
        y as i16,
        m as i8,
        d as i8,
        (s / 3600) as i8,
        (s / 60 % 60) as i8,
        (s % 60) as i8,
        (rest % MICROS_PER_SECOND * 1000) as i32,
    )
    .expect("the window is within jiff's range")
}

/// Returns the civil time of instant `i` in zone `zone`.
pub fn in_zone(i: i64, zone: &str) -> R<i64> {
    let tz = lookup(zone)?;
    i.checked_add(offset_at(&tz, i))
        .ok_or(Abort::Fault(Fault::ArithmeticOverflow))
}

/// Returns the civil time `civil` in zone `zone` resolves to, and the instant it names. A civil
/// time in an overlap names the earlier instant; one in a gap moves forward by the length of the
/// gap when `clamp` holds and resolves to `None` otherwise.
pub fn resolve(civil: i64, zone: &str, clamp: bool) -> R<Option<(i64, i64)>> {
    let tz = lookup(zone)?;
    let overflow = Abort::Fault(Fault::ArithmeticOverflow);
    let at = |off: i64| civil.checked_sub(off).ok_or(overflow.clone());
    Ok(
        match tz
            .to_ambiguous_timestamp(jiff_civil(into_window(civil)))
            .offset()
        {
            AmbiguousOffset::Unambiguous { offset } => Some((civil, at(seconds(offset))?)),
            AmbiguousOffset::Fold { before, after } => {
                let i = at(seconds(before).max(seconds(after)))?;
                Some((civil, i))
            }
            AmbiguousOffset::Gap { .. } if !clamp => None,
            AmbiguousOffset::Gap { before, after } => {
                let moved = civil
                    .checked_add(seconds(after) - seconds(before))
                    .ok_or(Abort::Fault(Fault::NoSuchDate))?;
                Some((moved, at(seconds(before))?))
            }
        },
    )
}

/// Returns the instant zoned civil time `civil` in `zone` names, a civil time in a gap resolved
/// forward.
pub fn to_instant(civil: i64, zone: &str) -> R<i64> {
    Ok(resolve(civil, zone, true)?
        .expect("a clamped civil time resolves")
        .1)
}
