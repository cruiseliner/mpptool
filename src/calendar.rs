//! Working calendars for scheduling: a weekly work pattern (any days, any shifts — 6/7-day weeks, 10-hour days,
//! Saturday overtime shifts), public holidays by country, and custom exception days from a file.
//!
//! A [`CalendarSpec`] describes the calendar; [`Timeline`] turns it into a continuous axis of *working minutes* so the
//! scheduler can add durations and lags and map positions back to timestamps.
use crate::holidays;
use crate::model::{CalendarRec, DayRec, ExceptionRec};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use std::collections::BTreeMap;

/// Working time ranges of one day, in minutes after midnight (`end` may be 1440).
pub type Ranges = Vec<(u32, u32)>;

/// One custom calendar day: no ranges = non-working (holiday, shutdown); ranges = working with these hours
/// (overtime Saturday, make-up day, extended shift).
#[derive(Debug, Clone, PartialEq)]
pub struct CustomDay {
    pub date: NaiveDate,
    pub name: Option<String>,
    pub ranges: Ranges,
}

/// Everything that defines a project calendar.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarSpec {
    /// Index 0 = Sunday … 6 = Saturday. An empty list is a non-working day.
    pub week: [Ranges; 7],
    /// Built-in holiday countries, comma-separated (e.g. `US`, `US,CA`); empty for none.
    pub holidays: String,
    /// Years whose holidays apply (and are written as exceptions). `None`: every year the schedule needs.
    pub years: Option<(i32, i32)>,
    /// Custom days; they override the weekly pattern and built-in holidays.
    pub custom: Vec<CustomDay>,
    /// Working hours in one duration "day" (CSV `duration_days` and `d` lags). Default 8.
    pub hours_per_day: f64,
    /// Calendar name written to MSPDI.
    pub name: String,
}

const STD: [(u32, u32); 2] = [(480, 720), (780, 1020)];

impl Default for CalendarSpec {
    /// Microsoft Project's Standard calendar: Mon–Fri 08:00–12:00, 13:00–17:00, no holidays, 8 h/day.
    fn default() -> Self {
        let w = STD.to_vec();
        CalendarSpec {
            week: [vec![], w.clone(), w.clone(), w.clone(), w.clone(), w, vec![]],
            holidays: String::new(),
            years: None,
            custom: vec![],
            hours_per_day: 8.0,
            name: "Standard".into(),
        }
    }
}

fn hm(s: &str) -> Result<u32, String> {
    let s = s.trim();
    let (h, m) = s.split_once(':').unwrap_or((s, "0"));
    match (h.trim().parse::<u32>(), m.trim().parse::<u32>()) {
        (Ok(h), Ok(m)) if m < 60 && (h < 24 || (h == 24 && m == 0)) => Ok(h * 60 + m),
        _ => Err(format!("bad time '{s}' (want HH:MM)")),
    }
}

/// Parse working ranges such as `08:00-12:00,13:00-17:00` (also `;` separated). `off`/`none`/empty = no work.
pub fn parse_ranges(s: &str) -> Result<Ranges, String> {
    let s = s.trim();
    if s.is_empty() || ["off", "none", "nonworking", "-"].contains(&s.to_lowercase().as_str()) {
        return Ok(vec![]);
    }
    let mut v: Ranges = vec![];
    for part in s.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = part.split_once('-').ok_or(format!("bad time range '{part}' (want HH:MM-HH:MM)"))?;
        let (a, b) = (hm(a)?, hm(b)?);
        if b <= a {
            return Err(format!("time range '{part}' must end after it starts (overnight shifts: split at midnight)"));
        }
        v.push((a, b));
    }
    v.sort();
    if v.windows(2).any(|w| w[1].0 < w[0].1) {
        return Err(format!("time ranges '{s}' overlap"));
    }
    Ok(v)
}

fn weekday_index(s: &str) -> Result<usize, String> {
    let k = s.trim().to_lowercase();
    let names = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
    names.iter().position(|n| k.starts_with(n) && k.len() >= 2).ok_or(format!("bad weekday '{s}' (want mon, tue, … sun)"))
}

/// Parse a weekday set: `mon-fri`, `mon-sat`, `sun-sat`, `all`, `mon,tue,thu`, `fri-mon` (wraps). Index 0 = Sunday.
pub fn parse_days(s: &str) -> Result<[bool; 7], String> {
    let mut out = [false; 7];
    let s = s.trim().to_lowercase();
    if s == "all" || s == "7" {
        return Ok([true; 7]);
    }
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some((a, b)) = part.split_once('-') {
            let (mut i, j) = (weekday_index(a)?, weekday_index(b)?);
            loop {
                out[i] = true;
                if i == j {
                    break;
                }
                i = (i + 1) % 7;
            }
        } else {
            out[weekday_index(part)?] = true;
        }
    }
    if !out.iter().any(|d| *d) {
        return Err(format!("no working days in '{s}'"));
    }
    Ok(out)
}

/// Parse `DAYS=RANGES`, e.g. `sat=07:00-12:00`, `mon-thu=06:00-16:30`, `fri=off`.
pub fn parse_day_override(s: &str) -> Result<([bool; 7], Ranges), String> {
    let (d, r) = s.split_once('=').ok_or(format!("bad --day '{s}' (want e.g. sat=07:00-12:00 or fri=off)"))?;
    Ok((parse_days(d)?, parse_ranges(r)?))
}

/// Parse a custom holiday / exception file. One entry per line, comma-separated:
/// `date[..date], name[, hours]` — `hours` empty or `off` = non-working, `HH:MM-HH:MM[;…]` = working with those hours,
/// `work` = working with `default_hours`. Blank lines, `#` comments and a `date,…` header line are skipped.
pub fn parse_holiday_file(text: &str, default_hours: &Ranges) -> Result<Vec<CustomDay>, String> {
    let mut out = vec![];
    let mut rdr =
        csv::ReaderBuilder::new().has_headers(false).flexible(true).trim(csv::Trim::All).comment(Some(b'#')).from_reader(text.as_bytes());
    for (n, rec) in rdr.records().enumerate() {
        let rec = rec.map_err(|e| format!("holiday file line {}: {e}", n + 1))?;
        let f = |i: usize| rec.get(i).unwrap_or("").trim();
        let ds = f(0);
        if ds.is_empty() || ds.eq_ignore_ascii_case("date") {
            continue;
        }
        let line = rec.position().map(|p| p.line()).unwrap_or(n as u64 + 1);
        let pd = |s: &str| {
            NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
                .map_err(|_| format!("holiday file line {line}: bad date '{s}' (want YYYY-MM-DD)"))
        };
        let (a, b) = match ds.split_once("..") {
            Some((a, b)) => (pd(a)?, pd(b)?),
            None => (pd(ds)?, pd(ds)?),
        };
        if b < a || (b - a).num_days() > 3660 {
            return Err(format!("holiday file line {line}: bad date range '{ds}'"));
        }
        let name = Some(f(1).to_string()).filter(|s| !s.is_empty());
        let h = f(2);
        let ranges = if h.eq_ignore_ascii_case("work") || h.eq_ignore_ascii_case("working") {
            default_hours.clone()
        } else {
            parse_ranges(h).map_err(|e| format!("holiday file line {line}: {e}"))?
        };
        let mut d = a;
        while d <= b {
            out.push(CustomDay { date: d, name: name.clone(), ranges: ranges.clone() });
            d = d.succ_opt().ok_or("date overflow")?;
        }
    }
    Ok(out)
}

impl CalendarSpec {
    /// Start from a weekday set with the same hours every working day.
    pub fn with_week(days: [bool; 7], hours: &Ranges) -> Self {
        let mut s = CalendarSpec::default();
        for (i, on) in days.iter().enumerate() {
            s.week[i] = if *on { hours.clone() } else { vec![] };
        }
        s
    }

    /// True when this is exactly the default Standard calendar (output identical to earlier versions).
    pub fn is_standard(&self) -> bool {
        *self == CalendarSpec::default()
    }

    /// Typical working day hours (the first working weekday's ranges), used for `work` entries.
    pub fn default_hours(&self) -> Ranges {
        self.week.iter().find(|r| !r.is_empty()).cloned().unwrap_or_else(|| STD.to_vec())
    }

    pub fn minutes_per_day(&self) -> f64 {
        self.hours_per_day * 60.0
    }

    /// Validate and resolve holidays into a [`WorkCalendar`]. `start_year` anchors the holiday window when `years`
    /// is not given.
    pub fn resolve(&self, start_year: i32) -> Result<WorkCalendar, String> {
        if self.week.iter().all(|r| r.is_empty()) {
            return Err("calendar has no working weekdays".into());
        }
        if !(self.hours_per_day > 0.0 && self.hours_per_day <= 24.0) {
            return Err(format!("hours per day must be between 0 and 24 (got {})", self.hours_per_day));
        }
        let (y0, y1) = self.years.unwrap_or((start_year - 1, start_year + 60));
        let mut days: BTreeMap<NaiveDate, (Option<String>, Ranges)> = BTreeMap::new();
        for h in holidays::for_years(&self.holidays, y0, y1)? {
            days.entry(h.date)
                .and_modify(|e| {
                    if let Some(n) = e.0.as_mut().filter(|n| !n.contains(&h.name)) {
                        n.push_str(" / ");
                        n.push_str(&h.name);
                    }
                })
                .or_insert((Some(h.name), vec![]));
        }
        for c in &self.custom {
            days.insert(c.date, (c.name.clone(), c.ranges.clone()));
        }
        Ok(WorkCalendar { week: self.week.clone(), days })
    }
}

/// A resolved calendar: weekly pattern plus dated exceptions.
#[derive(Debug, Clone)]
pub struct WorkCalendar {
    pub week: [Ranges; 7],
    /// Dated exceptions (holidays and custom days), overriding the weekly pattern.
    pub days: BTreeMap<NaiveDate, (Option<String>, Ranges)>,
}

impl WorkCalendar {
    /// Working ranges on date `d`.
    pub fn ranges(&self, d: NaiveDate) -> &[(u32, u32)] {
        match self.days.get(&d) {
            Some((_, r)) => r,
            None => &self.week[d.weekday().num_days_from_sunday() as usize],
        }
    }

    pub fn is_work(&self, d: NaiveDate) -> bool {
        !self.ranges(d).is_empty()
    }

    /// The MSPDI calendar: weekly pattern and every exception dated within `from..=to` that differs from the
    /// weekly pattern.
    pub fn to_rec(&self, uid: i32, name: &str, from: NaiveDate, to: NaiveDate) -> CalendarRec {
        let t = |m: u32| {
            if m >= 1440 { NaiveTime::from_hms_opt(0, 0, 0) } else { NaiveTime::from_hms_opt(m / 60, m % 60, 0) }.expect("valid time")
        };
        let tr = |r: &Ranges| r.iter().map(|&(a, b)| (t(a), t(b))).collect::<Vec<_>>();
        let days = std::array::from_fn(|i| DayRec { working: Some(!self.week[i].is_empty()), ranges: tr(&self.week[i]) });
        let exceptions = self
            .days
            .range(from..=to)
            .filter(|(d, (_, r))| *r != self.week[d.weekday().num_days_from_sunday() as usize])
            .map(|(d, (n, r))| ExceptionRec { name: n.clone(), from: *d, to: *d, working: !r.is_empty(), ranges: tr(r) })
            .collect();
        CalendarRec { uid, name: name.to_string(), base_uid: None, days, exceptions }
    }
}

struct TDay {
    date: NaiveDate,
    ranges: Ranges,
    cum: f64,
    len: f64,
}

/// A continuous working-minute axis starting at the first working moment on or after `base`.
pub struct Timeline {
    cal: WorkCalendar,
    days: Vec<TDay>,
    next: NaiveDate,
}

impl Timeline {
    pub fn new(cal: WorkCalendar, base: NaiveDate) -> Self {
        let mut t = Timeline { cal, days: vec![], next: base };
        t.extend();
        t
    }

    pub fn calendar(&self) -> &WorkCalendar {
        &self.cal
    }

    /// The first working date of the axis (position 0).
    pub fn base(&self) -> NaiveDate {
        self.days[0].date
    }

    fn total(&self) -> f64 {
        self.days.last().map(|d| d.cum + d.len).unwrap_or(0.0)
    }

    /// Append the next working day. Terminates because the weekly pattern has at least one working day and
    /// dated exceptions are finite.
    fn extend(&mut self) {
        loop {
            let d = self.next;
            self.next = d.succ_opt().expect("date overflow");
            let r = self.cal.ranges(d).to_vec();
            if !r.is_empty() {
                let len = r.iter().map(|(a, b)| (b - a) as f64).sum();
                let cum = self.total();
                self.days.push(TDay { date: d, ranges: r, cum, len });
                return;
            }
        }
    }

    /// Working-minute position of the start of the first working day on or after `d` (0 before the base).
    pub fn pos_of_date(&mut self, d: NaiveDate) -> f64 {
        while self.days.last().is_some_and(|x| x.date < d) {
            self.extend();
        }
        let i = self.days.partition_point(|x| x.date < d);
        self.days[i].cum
    }

    /// First working date on or after `d` and the time work starts on it.
    pub fn first_work_at(&mut self, d: NaiveDate) -> NaiveDateTime {
        let p = self.pos_of_date(d);
        self.to_dt(p, false)
    }

    /// Timestamp of working-minute position `m`. For finishes a position on a day boundary maps to the end of the
    /// previous working day; within a day, a position at the end of a range (e.g. lunch) maps to that range end.
    pub fn to_dt(&mut self, m: f64, is_finish: bool) -> NaiveDateTime {
        let m = m.max(0.0);
        while self.total() < m + if is_finish { 0.0 } else { 1e-6 } {
            self.extend();
        }
        let i = if is_finish && m > 1e-6 {
            self.days.partition_point(|x| x.cum + x.len < m - 1e-6)
        } else {
            self.days.partition_point(|x| x.cum + x.len <= m + 1e-6)
        };
        let day = &self.days[i.min(self.days.len() - 1)];
        let mut off = (m - day.cum).round().max(0.0) as u32;
        let mut at = day.ranges[0].0;
        for &(a, b) in &day.ranges {
            if off <= b - a {
                at = a + off;
                break;
            }
            off -= b - a;
            at = b;
        }
        let t = if at >= 1440 {
            return day.date.succ_opt().expect("date overflow").and_hms_opt(0, 0, 0).expect("valid time");
        } else {
            NaiveTime::from_hms_opt(at / 60, at % 60, 0).expect("valid time")
        };
        day.date.and_time(t)
    }

    /// Working-minute position of a timestamp (time-of-day honoured; non-working time rolls forward).
    pub fn pos_of_dt(&mut self, dt: NaiveDateTime) -> f64 {
        let p = self.pos_of_date(dt.date());
        let i = self.days.partition_point(|x| x.cum < p - 1e-6);
        let day = &self.days[i];
        if day.date != dt.date() {
            return p;
        }
        let tm = dt.time().hour() * 60 + dt.time().minute();
        let mut acc = 0.0;
        for &(a, b) in &day.ranges {
            if tm <= a {
                break;
            }
            acc += (tm.min(b) - a) as f64;
        }
        p + acc
    }
}

/// The project calendar of a parsed plan (default calendar, else the first, else Standard) as a [`CalendarSpec`].
pub fn spec_from_plan(plan: &crate::model::Plan) -> CalendarSpec {
    let hpd = if plan.minutes_per_day > 0 { plan.minutes_per_day as f64 / 60.0 } else { 8.0 };
    let mut spec = CalendarSpec { hours_per_day: hpd, ..Default::default() };
    let cal = plan.default_calendar_uid.and_then(|u| plan.calendars.iter().find(|c| c.uid == u)).or(plan.calendars.first());
    if let Some(c) = cal {
        let m = |t: &NaiveTime| t.hour() * 60 + t.minute();
        let conv = |r: &[(NaiveTime, NaiveTime)]| -> Ranges {
            r.iter().map(|(a, b)| (m(a), if m(b) == 0 { 1440 } else { m(b) })).filter(|(a, b)| b > a).collect()
        };
        for (i, d) in c.days.iter().enumerate() {
            spec.week[i] = match d.working {
                Some(false) => vec![],
                Some(true) if !d.ranges.is_empty() => conv(&d.ranges),
                _ => spec.week[i].clone(),
            };
        }
        if spec.week.iter().all(|w| w.is_empty()) {
            spec.week = CalendarSpec::default().week;
        }
        for e in &c.exceptions {
            let mut d = e.from;
            while d <= e.to && (d - e.from).num_days() <= 3660 {
                spec.custom.push(CustomDay { date: d, name: e.name.clone(), ranges: if e.working { conv(&e.ranges) } else { vec![] } });
                match d.succ_opt() {
                    Some(n) => d = n,
                    None => break,
                }
            }
        }
        spec.name = c.name.clone();
    }
    spec
}

/// The resolved project calendar of a plan; falls back to Standard (Mon–Fri) if the file's calendar is unusable.
pub fn plan_calendar(plan: &crate::model::Plan) -> WorkCalendar {
    let y = plan.start.map(|d| d.year()).unwrap_or(2000);
    spec_from_plan(plan).resolve(y).or_else(|_| CalendarSpec::default().resolve(y)).expect("standard calendar resolves")
}

impl WorkCalendar {
    /// Signed number of working days from `a` to `b`: working days `d` with `a < d <= b` (0 on the same day).
    pub fn working_days_between(&self, a: NaiveDate, b: NaiveDate) -> i64 {
        if b < a {
            return -self.working_days_between(b, a);
        }
        let mut n = 0;
        let mut d = a;
        while d < b {
            d = match d.succ_opt() {
                Some(x) => x,
                None => break,
            };
            if self.is_work(d) {
                n += 1;
            }
        }
        n
    }

    /// Working minutes on date `d`.
    pub fn minutes_on(&self, d: NaiveDate) -> f64 {
        self.ranges(d).iter().map(|(a, b)| (b - a) as f64).sum()
    }

    /// Working minutes between two timestamps (0 when `b <= a`).
    pub fn minutes_between(&self, a: NaiveDateTime, b: NaiveDateTime) -> f64 {
        if b <= a {
            return 0.0;
        }
        let mut t = Timeline::new(self.clone(), a.date());
        (t.pos_of_dt(b) - t.pos_of_dt(a)).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn parses_days_and_ranges() {
        assert_eq!(parse_days("mon-fri").unwrap(), [false, true, true, true, true, true, false]);
        assert_eq!(parse_days("mon-sat").unwrap(), [false, true, true, true, true, true, true]);
        assert_eq!(parse_days("fri-mon").unwrap(), [true, true, false, false, false, true, true]);
        assert_eq!(parse_days("all").unwrap(), [true; 7]);
        assert_eq!(parse_days("Tue,Thursday").unwrap(), [false, false, true, false, true, false, false]);
        assert!(parse_days("xyz").is_err());
        assert_eq!(parse_ranges("06:00-10:00, 10:30-16:30").unwrap(), vec![(360, 600), (630, 990)]);
        assert_eq!(parse_ranges("off").unwrap(), vec![]);
        assert!(parse_ranges("10:00-09:00").is_err());
        assert!(parse_ranges("08:00-12:00,11:00-13:00").is_err());
        assert!(parse_ranges("25:00-26:00").is_err());
        let (days, r) = parse_day_override("sat=07:00-12:00").unwrap();
        assert!(days[6] && !days[0]);
        assert_eq!(r, vec![(420, 720)]);
    }

    #[test]
    fn holiday_file() {
        let txt = "date,name,hours\n# shutdown\n2026-12-24..2026-12-26,Winter shutdown\n2026-11-28,OT Saturday,07:00-15:00\n2026-12-05,Make-up,work\n";
        let v = parse_holiday_file(txt, &STD.to_vec()).unwrap();
        assert_eq!(v.len(), 5);
        assert!(v[0].ranges.is_empty() && v[0].name.as_deref() == Some("Winter shutdown"));
        assert_eq!(v[3].ranges, vec![(420, 900)]);
        assert_eq!(v[4].ranges, STD.to_vec());
        assert!(parse_holiday_file("2026-13-01,x\n", &vec![]).unwrap_err().contains("line 1"));
    }

    #[test]
    fn timeline_skips_holidays_and_honours_overtime() {
        let mut spec = CalendarSpec { holidays: "US".into(), ..Default::default() };
        spec.week[6] = vec![(420, 720)]; // Saturday OT 07:00-12:00
        let cal = spec.resolve(2026).unwrap();
        assert!(!cal.is_work(d("2026-11-26"))); // Thanksgiving
        assert!(cal.is_work(d("2026-11-28"))); // Saturday OT
        let mut t = Timeline::new(cal, d("2026-11-25"));
        assert_eq!(t.to_dt(480.0, true).to_string(), "2026-11-25 17:00:00");
        // Thursday is a holiday, Friday is a full day, Saturday is 5 hours.
        assert_eq!(t.to_dt(480.0, false).to_string(), "2026-11-27 08:00:00");
        assert_eq!(t.to_dt(960.0 + 300.0, true).to_string(), "2026-11-28 12:00:00");
        assert_eq!(t.to_dt(960.0 + 300.0, false).to_string(), "2026-11-30 08:00:00");
        assert_eq!(t.pos_of_date(d("2026-11-26")), 480.0);
        assert_eq!(t.pos_of_dt(d("2026-11-27").and_hms_opt(14, 0, 0).unwrap()), 480.0 + 300.0);
        let rec = t.calendar().to_rec(1, "Project", d("2026-01-01"), d("2026-12-31"));
        assert_eq!(rec.exceptions.len(), 11);
        assert!(rec.days[6].working == Some(true) && rec.days[0].working == Some(false));
    }

    #[test]
    fn rejects_bad_calendars() {
        let spec = CalendarSpec::with_week([false; 7], &STD.to_vec());
        assert!(spec.resolve(2026).is_err());
        assert!(CalendarSpec { hours_per_day: 0.0, ..Default::default() }.resolve(2026).is_err());
        assert!(CalendarSpec { holidays: "ZZ".into(), ..Default::default() }.resolve(2026).is_err());
        assert!(CalendarSpec::default().is_standard());
    }
}
