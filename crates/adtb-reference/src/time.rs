pub use adtb_lang::calendar::{
    MICROS_PER_DAY, at_day, civil_from_days, date_in_range, days_from_civil, days_in_month,
};

/// Returns the date `months` months and then `days` days after `date`; `None` when the month
/// arithmetic lands on a day the month lacks and `clamp` is false.
pub fn add_period(date: i64, months: i32, days: i32, clamp: bool) -> Option<i64> {
    let (y, m, d) = civil_from_days(date);
    let total = y * 12 + (i64::from(m) - 1) + i64::from(months);
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
    Some(days_from_civil(ny, nm, nd) + i64::from(days))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
