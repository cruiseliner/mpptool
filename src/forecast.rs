//! Probabilistic "when will it be done?" forecasting from throughput history (Monte Carlo resampling of completed
//! items per period, as popularised by Vacanti and Magennis for flow/agile teams). Works for any countable work:
//! user stories, submittals reviewed, RFIs closed, punch-list items, rooms finished.
use crate::calendar::WorkCalendar;
use crate::lps::{Table, get};
use crate::risk::Rng;
use chrono::NaiveDate;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Period {
    /// One sample = one working day on the calendar.
    Day,
    /// One sample = one calendar week.
    Week,
}

#[derive(Debug, Clone)]
pub struct ForecastOptions {
    pub remaining: f64,
    pub start: NaiveDate,
    pub period: Period,
    pub runs: usize,
    pub seed: u64,
    /// Scope growth / split factor range (low, high), sampled uniformly per run and applied to `remaining`.
    pub split: (f64, f64),
    /// Also forecast how many items are done by this date.
    pub by: Option<NaiveDate>,
    /// Use only the last N samples (recent history).
    pub last: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct WhenPoint {
    pub confidence: f64,
    pub periods: usize,
    pub date: NaiveDate,
}

#[derive(Debug, Serialize)]
pub struct HowManyPoint {
    pub confidence: f64,
    /// At least this many items done by the date, with this confidence.
    pub items: f64,
}

#[derive(Debug, Serialize)]
pub struct Forecast {
    pub period: Period,
    pub samples: usize,
    pub zero_periods: usize,
    pub mean_throughput: f64,
    pub remaining: f64,
    pub split: (f64, f64),
    pub start: NaiveDate,
    pub runs: usize,
    pub seed: u64,
    pub when: Vec<WhenPoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub how_many: Vec<HowManyPoint>,
    pub warnings: Vec<String>,
}

/// History CSV: a `completed` (or `throughput`/`count`/`done`) column, one row per period (oldest first); an optional
/// `date` column sorts the rows. A column of item finish dates (`finished`/`done_date`, one row per item) is also
/// accepted and binned into periods, including the empty periods.
pub fn parse_history(text: &str, period: Period, cal: &WorkCalendar) -> Result<Vec<f64>, String> {
    let t = Table::parse(text, "throughput")?;
    if let Some(cf) = t.col(&["finished", "done_date", "completed_date", "resolved", "closed"]) {
        let mut ds = vec![];
        for (line, r) in &t.recs {
            if let Some(d) = crate::lps::date(get(r, Some(cf)), *line, "finish date")? {
                ds.push(d);
            }
        }
        ds.sort();
        let (Some(a), Some(b)) = (ds.first().copied(), ds.last().copied()) else { return Err("no finish dates".into()) };
        let mut bins: Vec<f64> = vec![];
        let mut keys: Vec<NaiveDate> = vec![];
        let key = |d: NaiveDate| match period {
            Period::Week => d - chrono::Duration::days(chrono::Datelike::weekday(&d).num_days_from_monday() as i64),
            Period::Day => d,
        };
        let mut d = key(a);
        while d <= b {
            if period == Period::Week || cal.is_work(d) {
                keys.push(d);
                bins.push(0.0);
            }
            d += chrono::Duration::days(if period == Period::Week { 7 } else { 1 });
        }
        for x in ds {
            let k = key(x);
            match keys.iter().position(|y| *y == k) {
                Some(i) => bins[i] += 1.0,
                None => {
                    // Finished on a non-working day: count it on the next working day.
                    if let Some(i) = keys.iter().position(|y| *y > k) {
                        bins[i] += 1.0
                    }
                }
            }
        }
        return Ok(bins);
    }
    let cc = t
        .col(&["completed", "throughput", "count", "done", "items"])
        .ok_or("throughput CSV needs a 'completed' column (or 'finished' dates)")?;
    let cd = t.col(&["date", "period", "week", "day"]);
    let mut v: Vec<(String, f64)> = vec![];
    for (line, r) in &t.recs {
        let x = get(r, Some(cc))
            .parse::<f64>()
            .ok()
            .filter(|x| x.is_finite() && *x >= 0.0)
            .ok_or(format!("throughput line {line}: bad completed '{}'", get(r, Some(cc))))?;
        v.push((get(r, cd).to_string(), x));
    }
    if cd.is_some() && v.iter().all(|x| NaiveDate::parse_from_str(&x.0, "%Y-%m-%d").is_ok()) {
        v.sort_by(|a, b| a.0.cmp(&b.0));
    }
    Ok(v.into_iter().map(|x| x.1).collect())
}

pub fn forecast(history: &[f64], cal: &WorkCalendar, o: &ForecastOptions) -> Result<Forecast, String> {
    let h: Vec<f64> = match o.last {
        Some(n) if n > 0 && n < history.len() => history[history.len() - n..].to_vec(),
        _ => history.to_vec(),
    };
    if h.is_empty() {
        return Err("throughput history is empty".into());
    }
    if h.iter().all(|x| *x <= 0.0) {
        return Err("throughput history has no completed items: cannot forecast".into());
    }
    if !(o.remaining.is_finite() && o.remaining > 0.0) {
        return Err("--remaining must be > 0".into());
    }
    if !(o.split.0 > 0.0 && o.split.0 <= o.split.1 && o.split.1.is_finite()) {
        return Err("--split needs 0 < low <= high".into());
    }
    if o.runs == 0 || o.runs > 1_000_000 {
        return Err("runs must be 1..=1000000".into());
    }
    let mean = h.iter().sum::<f64>() / h.len() as f64;
    // Cap the simulated horizon so a near-zero history cannot loop for ever.
    let cap = ((o.remaining * o.split.1 / mean) * 20.0).ceil().max(1000.0) as usize;
    let mut rng = Rng::new(o.seed);
    let mut periods = Vec::with_capacity(o.runs);
    let by_periods = o.by.map(|b| periods_until(o.start, b, o.period, cal));
    let mut counts = vec![];
    for _ in 0..o.runs {
        let f = o.split.0 + (o.split.1 - o.split.0) * rng.uniform();
        let target = o.remaining * f;
        let mut done = 0.0;
        let mut p = 0;
        while done + 1e-9 < target && p < cap {
            done += h[((rng.uniform() * h.len() as f64) as usize).min(h.len() - 1)];
            p += 1;
        }
        periods.push(p);
        if let Some(bp) = by_periods {
            let mut c = 0.0;
            for _ in 0..bp {
                c += h[((rng.uniform() * h.len() as f64) as usize).min(h.len() - 1)];
            }
            counts.push(c);
        }
    }
    periods.sort();
    counts.sort_by(f64::total_cmp);
    let q = |x: f64| periods[((x * o.runs as f64) as usize).min(o.runs - 1)];
    let when = [0.5, 0.7, 0.85, 0.95]
        .iter()
        .map(|&c| {
            let p = q(c);
            WhenPoint { confidence: c, periods: p, date: date_after(o.start, p, o.period, cal) }
        })
        .collect();
    let how_many = if counts.is_empty() {
        vec![]
    } else {
        [0.5, 0.7, 0.85, 0.95]
            .iter()
            .map(|&c| HowManyPoint { confidence: c, items: counts[(((1.0 - c) * o.runs as f64) as usize).min(o.runs - 1)] })
            .collect()
    };
    let mut warnings = vec![];
    if h.len() < 10 {
        warnings.push(format!("only {} samples of history; use at least 10-20 periods for a stable forecast", h.len()));
    }
    if periods.last().is_some_and(|p| *p >= cap) {
        warnings.push("some runs hit the horizon cap; history throughput is very low".into());
    }
    Ok(Forecast {
        period: o.period,
        samples: h.len(),
        zero_periods: h.iter().filter(|x| **x <= 0.0).count(),
        mean_throughput: mean,
        remaining: o.remaining,
        split: o.split,
        start: o.start,
        runs: o.runs,
        seed: o.seed,
        when,
        by: o.by,
        how_many,
        warnings,
    })
}

/// End date of the `p`-th period starting at `start` (period 1 ends on the first working day / 6 days after start).
fn date_after(start: NaiveDate, p: usize, period: Period, cal: &WorkCalendar) -> NaiveDate {
    match period {
        Period::Week => start + chrono::Duration::days(7 * p as i64 - 1).max(chrono::Duration::days(0)),
        Period::Day => {
            let mut d = start;
            let mut n = 0;
            let mut guard = 0;
            loop {
                if cal.is_work(d) {
                    n += 1;
                    if n >= p.max(1) {
                        return d;
                    }
                }
                d = d.succ_opt().unwrap_or(d);
                guard += 1;
                if guard > 400_000 {
                    return d;
                }
            }
        }
    }
}

/// Number of whole periods from `start` through `by` inclusive.
fn periods_until(start: NaiveDate, by: NaiveDate, period: Period, cal: &WorkCalendar) -> usize {
    if by < start {
        return 0;
    }
    match period {
        Period::Week => ((by - start).num_days() + 1) as usize / 7,
        Period::Day => {
            let mut d = start;
            let mut n = 0;
            while d <= by {
                n += cal.is_work(d) as usize;
                d = d.succ_opt().unwrap_or(by.succ_opt().unwrap_or(by));
            }
            n
        }
    }
}

pub fn render_text(f: &Forecast) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let unit = if f.period == Period::Week { "week" } else { "working day" };
    let _ = writeln!(
        s,
        "Throughput history: {} {unit}s, mean {:.2} items/{unit}, {} with zero; {} items remaining{} from {}",
        f.samples,
        f.mean_throughput,
        f.zero_periods,
        f.remaining,
        if f.split != (1.0, 1.0) { format!(" x split/growth {:.2}-{:.2}", f.split.0, f.split.1) } else { String::new() },
        f.start
    );
    let _ = writeln!(s, "When will it be done? ({} runs, seed {})", f.runs, f.seed);
    for w in &f.when {
        let _ = writeln!(s, "  {:>3.0}%: by {} ({} {unit}s)", w.confidence * 100.0, w.date, w.periods);
    }
    if let Some(b) = f.by {
        let _ = writeln!(s, "How many by {b}?");
        for h in &f.how_many {
            let _ = writeln!(s, "  {:>3.0}%: at least {:.0} items", h.confidence * 100.0, h.items);
        }
    }
    for w in &f.warnings {
        let _ = writeln!(s, "  WARNING: {w}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn constant_throughput_is_exact() {
        let cal = crate::calendar::CalendarSpec::default().resolve(2026).unwrap();
        let o = ForecastOptions {
            remaining: 20.0,
            start: d("2026-11-02"),
            period: Period::Week,
            runs: 500,
            seed: 1,
            split: (1.0, 1.0),
            by: Some(d("2026-11-29")),
            last: None,
        };
        let f = forecast(&[5.0; 12], &cal, &o).unwrap();
        assert!(f.when.iter().all(|w| w.periods == 4 && w.date == d("2026-11-29")));
        assert!(f.how_many.iter().all(|h| h.items == 20.0));
        let mut o2 = o.clone();
        o2.period = Period::Day;
        o2.by = None;
        let f2 = forecast(&[5.0; 12], &cal, &o2).unwrap();
        assert_eq!(f2.when[0].date, d("2026-11-05"), "4 working days: Mon..Thu");
        assert!(forecast(&[0.0, 0.0], &cal, &o).is_err());
        let v = forecast(&[0.0, 1.0, 2.0, 5.0, 0.0, 3.0, 1.0, 4.0, 2.0, 2.0], &cal, &o).unwrap();
        let p: Vec<usize> = v.when.iter().map(|w| w.periods).collect();
        assert!(p.windows(2).all(|x| x[0] <= x[1]), "{p:?}");
        let h = parse_history("finished\n2026-10-05\n2026-10-06\n2026-10-20\n", Period::Week, &cal).unwrap();
        assert_eq!(h, vec![2.0, 0.0, 1.0], "empty weeks count as zero");
        assert!(render_text(&v).contains("85%"));
    }
}
