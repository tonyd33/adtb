pub const MICROS_PER_DAY: i64 = 86_400_000_000;

/// Returns the days since 1970-01-01 of a proleptic Gregorian date.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Returns the year, month and day of a count of days since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        _ => 28,
    }
}

/// Returns the date `months` months and then `days` days after `date`; `None` when the month
/// arithmetic lands on a day the month lacks and `clamp` is false.
pub fn add_period(date: i64, months: i32, days: i32, clamp: bool) -> Option<i64> {
    let (y, m, d) = civil_from_days(date);
    let total = y * 12 + (m as i64 - 1) + months as i64;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
    let last = days_in_month(ny, nm);
    let nd = if d > last {
        if !clamp {
            return None;
        }
        last
    } else {
        d
    };
    Some(days_from_civil(ny, nm, nd) + days as i64)
}

/// Returns the microseconds of time of day `tod` on day `days`, or `None` outside an `I64`.
pub fn at_day(days: i64, tod: i64) -> Option<i64> {
    i64::try_from(i128::from(days) * i128::from(MICROS_PER_DAY) + i128::from(tod)).ok()
}

/// Returns whether a count of days names a date within the range of `Instant`.
pub fn date_in_range(days: i64) -> bool {
    let lo = i64::MIN.div_euclid(MICROS_PER_DAY);
    let hi = i64::MAX.div_euclid(MICROS_PER_DAY);
    lo <= days && days <= hi
}

pub fn show_date(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    if (0..=9999).contains(&y) {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!("{y:+06}-{m:02}-{d:02}")
    }
}

pub fn show_time(micros: i64) -> String {
    let s = micros / 1_000_000;
    format!(
        "{:02}:{:02}:{:02}.{:06}",
        s / 3600,
        s / 60 % 60,
        s % 60,
        micros % 1_000_000
    )
}

pub fn show_datetime(micros: i64) -> String {
    let days = micros.div_euclid(MICROS_PER_DAY);
    let rest = micros.rem_euclid(MICROS_PER_DAY);
    format!("{}T{}", show_date(days), show_time(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trips() {
        for z in [-800_000, -1, 0, 1, 59, 365, 11_016, 18_000, 2_932_896] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(z, days_from_civil(y, m, d));
        }
        assert_eq!((1970, 1, 1), civil_from_days(0));
        assert_eq!((2000, 2, 29), civil_from_days(days_from_civil(2000, 2, 29)));
    }

    #[test]
    fn month_arithmetic_clamps_or_refuses() {
        let jan31 = days_from_civil(2021, 1, 31);
        assert_eq!(
            Some(days_from_civil(2021, 2, 28)),
            add_period(jan31, 1, 0, true)
        );
        assert_eq!(None, add_period(jan31, 1, 0, false));
        assert_eq!(
            Some(days_from_civil(2022, 3, 2)),
            add_period(jan31, 13, 2, true)
        );
        assert_eq!("2021-01-31", show_date(jan31));
    }
}
