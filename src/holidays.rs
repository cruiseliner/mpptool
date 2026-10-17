//! Public-holiday rule tables for common countries, computed per year (no data files, no network).
//!
//! Supported codes: `US` (federal, with observed-date rules), `CA` (federal statutory), `GB` (England & Wales bank
//! holidays, with substitute days), `AU` (national), `DE` (nationwide), `FR`, `IN` (national gazetted, fixed-date only),
//! `MX` (Ley Federal del Trabajo, art. 74). One-off holidays proclaimed by governments (state funerals, jubilees) and
//! regional/state holidays are not included; add them with a custom holiday file.
use chrono::{Datelike, Duration, NaiveDate, Weekday};

/// One holiday occurrence. `date` is the day work stops (the observed date when the actual date falls on a weekend).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Holiday {
    pub date: NaiveDate,
    pub name: String,
    pub country: String,
}

/// Countries with built-in rules: `(code, description)`.
pub const COUNTRIES: &[(&str, &str)] = &[
    ("US", "United States federal holidays (5 U.S.C. 6103, observed Sat->Fri, Sun->Mon)"),
    ("CA", "Canada federal statutory holidays"),
    ("GB", "United Kingdom, England & Wales bank holidays"),
    ("AU", "Australia national public holidays"),
    ("DE", "Germany nationwide public holidays"),
    ("FR", "France jours fériés"),
    ("IN", "India national holidays (fixed-date gazetted only)"),
    ("MX", "Mexico statutory rest days (LFT art. 74)"),
];

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid holiday date")
}

/// The `n`-th (1-based) `wd` of the month, or the last one when `n == -1`.
fn nth_weekday(y: i32, m: u32, wd: Weekday, n: i32) -> NaiveDate {
    if n > 0 {
        let first = ymd(y, m, 1);
        let off = (7 + wd.num_days_from_monday() as i64 - first.weekday().num_days_from_monday() as i64) % 7;
        first + Duration::days(off + 7 * (n as i64 - 1))
    } else {
        let next = if m == 12 { ymd(y + 1, 1, 1) } else { ymd(y, m + 1, 1) };
        let last = next - Duration::days(1);
        let off = (7 + last.weekday().num_days_from_monday() as i64 - wd.num_days_from_monday() as i64) % 7;
        last - Duration::days(off)
    }
}

/// Gregorian Easter Sunday (anonymous Gregorian algorithm).
pub fn easter(y: i32) -> NaiveDate {
    let a = y % 19;
    let b = y / 100;
    let c = y % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = (h + l - 7 * m + 114) % 31 + 1;
    ymd(y, month as u32, day as u32)
}

/// US federal rule: Saturday -> Friday before, Sunday -> Monday after.
fn us_observed(d: NaiveDate) -> NaiveDate {
    match d.weekday() {
        Weekday::Sat => d - Duration::days(1),
        Weekday::Sun => d + Duration::days(1),
        _ => d,
    }
}

/// Weekend -> next Monday (Canada, Australia, UK single holidays).
fn monday_if_weekend(d: NaiveDate) -> NaiveDate {
    match d.weekday() {
        Weekday::Sat => d + Duration::days(2),
        Weekday::Sun => d + Duration::days(1),
        _ => d,
    }
}

/// Christmas and Boxing Day with substitute days (UK/AU/CA): both land on distinct weekdays.
fn christmas_boxing(y: i32) -> (NaiveDate, NaiveDate) {
    let xmas = ymd(y, 12, 25);
    match xmas.weekday() {
        Weekday::Fri => (xmas, ymd(y, 12, 28)),
        Weekday::Sat => (ymd(y, 12, 27), ymd(y, 12, 28)),
        Weekday::Sun => (ymd(y, 12, 27), ymd(y, 12, 26)),
        _ => (xmas, ymd(y, 12, 26)),
    }
}

/// Holidays defined for calendar year `y` (an observed date can fall in an adjacent year, e.g. US New Year's Day
/// 2028 is observed on Friday 2027-12-31). Errors on an unknown country code.
pub fn for_year(country: &str, y: i32) -> Result<Vec<Holiday>, String> {
    use Weekday::*;
    let code = country.trim().to_uppercase();
    let mut v: Vec<(NaiveDate, &str)> = vec![];
    match code.as_str() {
        "US" => {
            v.push((us_observed(ymd(y, 1, 1)), "New Year's Day"));
            if y >= 1986 {
                v.push((nth_weekday(y, 1, Mon, 3), "Birthday of Martin Luther King, Jr."));
            }
            v.push((nth_weekday(y, 2, Mon, 3), "Washington's Birthday"));
            v.push((nth_weekday(y, 5, Mon, -1), "Memorial Day"));
            if y >= 2021 {
                v.push((us_observed(ymd(y, 6, 19)), "Juneteenth National Independence Day"));
            }
            v.push((us_observed(ymd(y, 7, 4)), "Independence Day"));
            v.push((nth_weekday(y, 9, Mon, 1), "Labor Day"));
            v.push((nth_weekday(y, 10, Mon, 2), "Columbus Day"));
            v.push((us_observed(ymd(y, 11, 11)), "Veterans Day"));
            v.push((nth_weekday(y, 11, Thu, 4), "Thanksgiving Day"));
            v.push((us_observed(ymd(y, 12, 25)), "Christmas Day"));
        }
        "CA" => {
            let e = easter(y);
            v.push((monday_if_weekend(ymd(y, 1, 1)), "New Year's Day"));
            v.push((e - Duration::days(2), "Good Friday"));
            // Victoria Day: the Monday before May 25.
            let may24 = ymd(y, 5, 24);
            v.push((may24 - Duration::days(may24.weekday().num_days_from_monday() as i64), "Victoria Day"));
            v.push((monday_if_weekend(ymd(y, 7, 1)), "Canada Day"));
            v.push((nth_weekday(y, 9, Mon, 1), "Labour Day"));
            if y >= 2021 {
                v.push((monday_if_weekend(ymd(y, 9, 30)), "National Day for Truth and Reconciliation"));
            }
            v.push((nth_weekday(y, 10, Mon, 2), "Thanksgiving"));
            v.push((monday_if_weekend(ymd(y, 11, 11)), "Remembrance Day"));
            let (x, b) = christmas_boxing(y);
            v.push((x, "Christmas Day"));
            v.push((b, "Boxing Day"));
        }
        "GB" | "UK" => {
            let e = easter(y);
            v.push((monday_if_weekend(ymd(y, 1, 1)), "New Year's Day"));
            v.push((e - Duration::days(2), "Good Friday"));
            v.push((e + Duration::days(1), "Easter Monday"));
            v.push((nth_weekday(y, 5, Mon, 1), "Early May bank holiday"));
            v.push((nth_weekday(y, 5, Mon, -1), "Spring bank holiday"));
            v.push((nth_weekday(y, 8, Mon, -1), "Summer bank holiday"));
            let (x, b) = christmas_boxing(y);
            v.push((x, "Christmas Day"));
            v.push((b, "Boxing Day"));
        }
        "AU" => {
            let e = easter(y);
            v.push((monday_if_weekend(ymd(y, 1, 1)), "New Year's Day"));
            v.push((monday_if_weekend(ymd(y, 1, 26)), "Australia Day"));
            v.push((e - Duration::days(2), "Good Friday"));
            v.push((e + Duration::days(1), "Easter Monday"));
            v.push((ymd(y, 4, 25), "Anzac Day"));
            v.push((nth_weekday(y, 6, Mon, 2), "King's Birthday"));
            let (x, b) = christmas_boxing(y);
            v.push((x, "Christmas Day"));
            v.push((b, "Boxing Day"));
        }
        "DE" => {
            let e = easter(y);
            v.push((ymd(y, 1, 1), "Neujahr"));
            v.push((e - Duration::days(2), "Karfreitag"));
            v.push((e + Duration::days(1), "Ostermontag"));
            v.push((ymd(y, 5, 1), "Tag der Arbeit"));
            v.push((e + Duration::days(39), "Christi Himmelfahrt"));
            v.push((e + Duration::days(50), "Pfingstmontag"));
            v.push((ymd(y, 10, 3), "Tag der Deutschen Einheit"));
            v.push((ymd(y, 12, 25), "1. Weihnachtstag"));
            v.push((ymd(y, 12, 26), "2. Weihnachtstag"));
        }
        "FR" => {
            let e = easter(y);
            v.push((ymd(y, 1, 1), "Jour de l'an"));
            v.push((e + Duration::days(1), "Lundi de Pâques"));
            v.push((ymd(y, 5, 1), "Fête du Travail"));
            v.push((ymd(y, 5, 8), "Victoire 1945"));
            v.push((e + Duration::days(39), "Ascension"));
            v.push((e + Duration::days(50), "Lundi de Pentecôte"));
            v.push((ymd(y, 7, 14), "Fête nationale"));
            v.push((ymd(y, 8, 15), "Assomption"));
            v.push((ymd(y, 11, 1), "Toussaint"));
            v.push((ymd(y, 11, 11), "Armistice 1918"));
            v.push((ymd(y, 12, 25), "Noël"));
        }
        "IN" => {
            v.push((ymd(y, 1, 26), "Republic Day"));
            v.push((ymd(y, 8, 15), "Independence Day"));
            v.push((ymd(y, 10, 2), "Gandhi Jayanti"));
        }
        "MX" => {
            v.push((ymd(y, 1, 1), "Año Nuevo"));
            v.push((nth_weekday(y, 2, Mon, 1), "Día de la Constitución"));
            v.push((nth_weekday(y, 3, Mon, 3), "Natalicio de Benito Juárez"));
            v.push((ymd(y, 5, 1), "Día del Trabajo"));
            v.push((ymd(y, 9, 16), "Día de la Independencia"));
            if y >= 2024 && (y - 2024) % 6 == 0 {
                v.push((ymd(y, 10, 1), "Transmisión del Poder Ejecutivo Federal"));
            }
            v.push((nth_weekday(y, 11, Mon, 3), "Día de la Revolución"));
            v.push((ymd(y, 12, 25), "Navidad"));
        }
        _ => {
            let known: Vec<&str> = COUNTRIES.iter().map(|c| c.0).collect();
            return Err(format!("unknown holiday country '{country}' (built in: {}; or use --holiday-file)", known.join(", ")));
        }
    }
    let code = if code == "UK" { "GB".to_string() } else { code };
    let mut out: Vec<Holiday> = v.into_iter().map(|(date, n)| Holiday { date, name: n.to_string(), country: code.clone() }).collect();
    out.sort_by_key(|h| h.date);
    Ok(out)
}

/// Holidays of several countries (comma-separated codes, `none` for none) whose date lies in years `from..=to`.
pub fn for_years(countries: &str, from: i32, to: i32) -> Result<Vec<Holiday>, String> {
    let mut out = vec![];
    for c in countries.split([',', '+']).map(str::trim).filter(|c| !c.is_empty() && !c.eq_ignore_ascii_case("none")) {
        for y in from - 1..=to + 1 {
            out.extend(for_year(c, y)?.into_iter().filter(|h| (from..=to).contains(&h.date.year())));
        }
    }
    out.sort_by(|a, b| a.date.cmp(&b.date).then(a.country.cmp(&b.country)));
    Ok(out)
}

/// Parse a year range such as `2026`, `2026-2028` or `2026..2028`.
pub fn parse_years(s: &str) -> Result<(i32, i32), String> {
    let s = s.trim();
    let (a, b) = s.split_once("..").or_else(|| s.split_once('-')).unwrap_or((s, s));
    let p = |x: &str| x.trim().parse::<i32>().ok().filter(|y| (1900..=2200).contains(y));
    match (p(a), p(b)) {
        (Some(a), Some(b)) if a <= b => Ok((a, b)),
        _ => Err(format!("bad year range '{s}' (want e.g. 2026 or 2026-2028, years 1900-2200)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }
    fn dates(c: &str, y: i32) -> Vec<String> {
        for_year(c, y).unwrap().iter().map(|h| h.date.to_string()).collect()
    }

    #[test]
    fn us_federal_2026_matches_opm() {
        // OPM 2026: Jan 1, Jan 19, Feb 16, May 25, Jun 19, Jul 3 (observed), Sep 7, Oct 12, Nov 11, Nov 26, Dec 25.
        assert_eq!(
            dates("US", 2026),
            [
                "2026-01-01",
                "2026-01-19",
                "2026-02-16",
                "2026-05-25",
                "2026-06-19",
                "2026-07-03",
                "2026-09-07",
                "2026-10-12",
                "2026-11-11",
                "2026-11-26",
                "2026-12-25"
            ]
        );
    }

    #[test]
    fn us_observed_rules() {
        // 2027: Jun 19 Sat -> Fri Jun 18; Jul 4 Sun -> Mon Jul 5; Dec 25 Sat -> Fri Dec 24.
        let v = dates("US", 2027);
        for x in ["2027-06-18", "2027-07-05", "2027-12-24"] {
            assert!(v.contains(&x.to_string()), "{x} missing from {v:?}");
        }
        // New Year's Day 2028 is a Saturday: observed Friday 2027-12-31, which counts toward 2027.
        let y27 = for_years("US", 2027, 2027).unwrap();
        assert!(y27.iter().any(|h| h.date == d("2027-12-31") && h.name == "New Year's Day"));
        assert!(!for_years("US", 2028, 2028).unwrap().iter().any(|h| h.name == "New Year's Day"));
        assert!(!dates("US", 2020).iter().any(|x| x.ends_with("06-19")), "Juneteenth starts in 2021");
    }

    #[test]
    fn easter_and_european_tables() {
        assert_eq!(easter(2026), d("2026-04-05"));
        assert_eq!(easter(2027), d("2027-03-28"));
        assert_eq!(easter(2000), d("2000-04-23"));
        assert!(dates("DE", 2026).contains(&"2026-05-14".to_string())); // Ascension
        assert!(dates("FR", 2026).contains(&"2026-05-25".to_string())); // Whit Monday
        assert_eq!(dates("GB", 2026).len(), 8);
        // GB 2027: Christmas Sat -> Mon 27, Boxing Sun -> Tue 28.
        assert!(dates("GB", 2027).ends_with(&["2027-12-27".to_string(), "2027-12-28".to_string()]));
    }

    #[test]
    fn other_countries() {
        assert!(dates("CA", 2026).contains(&"2026-05-18".to_string())); // Victoria Day
        assert!(dates("CA", 2026).contains(&"2026-07-01".to_string()));
        assert!(dates("AU", 2026).contains(&"2026-01-26".to_string()));
        assert!(dates("MX", 2026).contains(&"2026-11-16".to_string()));
        assert!(dates("MX", 2030).contains(&"2030-10-01".to_string()));
        assert_eq!(dates("IN", 2026).len(), 3);
        assert!(for_year("XX", 2026).is_err());
        assert_eq!(for_years("US,CA", 2026, 2026).unwrap().len(), 11 + 10);
        assert!(for_years("none", 2026, 2026).unwrap().is_empty());
    }

    #[test]
    fn year_ranges() {
        assert_eq!(parse_years("2026").unwrap(), (2026, 2026));
        assert_eq!(parse_years("2026-2028").unwrap(), (2026, 2028));
        assert_eq!(parse_years("2026..2028").unwrap(), (2026, 2028));
        assert!(parse_years("2028-2026").is_err());
        assert!(parse_years("abc").is_err());
    }
}
