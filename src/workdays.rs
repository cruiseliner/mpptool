//! Small date helpers shared by the report commands.
use chrono::{Datelike, NaiveDate, NaiveDateTime, Weekday};

/// Signed number of Mon–Fri working days from `a` to `b` (0 when on the same day).
pub fn working_days_between(a: NaiveDate, b: NaiveDate) -> i64 {
    if b < a {
        return -working_days_between(b, a);
    }
    let days = (b - a).num_days();
    let weeks = days / 7;
    let mut n = weeks * 5;
    let mut d = a + chrono::Duration::days(weeks * 7);
    while d < b {
        d += chrono::Duration::days(1);
        if !matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
            n += 1;
        }
    }
    n
}

/// Working-day slip between two optional timestamps (`new - old`).
pub fn slip_days(old: Option<NaiveDateTime>, new: Option<NaiveDateTime>) -> Option<i64> {
    Some(working_days_between(old?.date(), new?.date()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }
    #[test]
    fn counts_weekdays() {
        assert_eq!(working_days_between(d("2026-03-06"), d("2026-03-09")), 1); // Fri -> Mon
        assert_eq!(working_days_between(d("2026-03-02"), d("2026-03-16")), 10);
        assert_eq!(working_days_between(d("2026-03-16"), d("2026-03-02")), -10);
        assert_eq!(working_days_between(d("2026-03-02"), d("2026-03-02")), 0);
    }
}
