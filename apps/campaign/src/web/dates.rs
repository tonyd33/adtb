use super::{Error, Result};
use adtb::typed::Date;
use axum::http::StatusCode;

pub(super) fn parse(value: &str) -> Result<Date> {
    let bad = || {
        Error::Http(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Enter a valid date as YYYY-MM-DD.",
        )
    };
    if value.len() != 10
        || !value.is_ascii()
        || value.as_bytes()[4] != b'-'
        || value.as_bytes()[7] != b'-'
        || !value
            .bytes()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7) || b.is_ascii_digit())
    {
        return Err(bad());
    }
    let mut fields = value.split('-');
    let year: i32 = fields.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    let month: i32 = fields.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    let day: i32 = fields.next().ok_or_else(bad)?.parse().map_err(|_| bad())?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    if fields.next().is_some() || !(1..=9999).contains(&year) || !(1..=days).contains(&day) {
        return Err(bad());
    }
    let prior = year - 1;
    let preceding = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]
        [usize::try_from(month - 1).map_err(|_| bad())?];
    Ok(Date(
        prior * 365 + prior / 4 - prior / 100
            + prior / 400
            + preceding
            + i32::from(leap && month > 2)
            + day
            - 1
            - 719162,
    ))
}

pub(super) fn display(date: Date) -> String {
    let z = i64::from(date.0) + 719468;
    let era = z.div_euclid(146097);
    let day_of_era = z - era * 146097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::{display, parse};
    #[test]
    fn date_boundaries() {
        assert_eq!(parse("1970-01-01").unwrap().0, 0);
        for value in [
            "0001-01-01",
            "1970-01-01",
            "2000-02-29",
            "2026-10-28",
            "9999-12-31",
        ] {
            assert_eq!(display(parse(value).unwrap()), value);
        }
        assert_eq!(
            parse("2000-03-01").unwrap().0 - parse("2000-02-28").unwrap().0,
            2
        );
        assert!(parse("1900-02-29").is_err());
        for bad in [
            "2026-02-30",
            "2026-00-01",
            "2026-13-01",
            "2026-01-00",
            "0000-01-01",
            "2026-1-001",
            "2026-01-1-",
            "+001-01-01",
            "2026-+1-01",
            "2026-01-+1",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
