//! Takt planning and location-based (flowline) scheduling from a trade × zone CSV.
//!
//! * `takt` mode: every trade (wagon) moves through the zones at one fixed takt time; trade `i` works zone `j` in takt
//!   period `i·(1+buffer) + j`. Total duration = (wagons + buffer wagons + zones − 1) × takt. Wagons whose work in a
//!   zone exceeds the takt are flagged (rebalance the zone or the crew).
//! * `flow` mode (line-of-balance / LBMS): each trade keeps one continuous crew and its own pace, starting as early as
//!   it can without catching up with the previous trade in any zone (plus `buffer` days).
//!
//! Durations are in working days; dates come from the working calendar (whole days).
use crate::calendar::WorkCalendar;
use crate::gantt::esc;
use crate::lps::{Table, get};
use chrono::NaiveDate;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Takt,
    Flow,
}

/// Work content: `trades[i]` in `zones[j]` takes `days[i][j]` working days.
#[derive(Debug, Clone)]
pub struct Input {
    pub trades: Vec<String>,
    pub zones: Vec<String>,
    pub days: Vec<Vec<f64>>,
}

/// Parse `trade,zone,days` (long form, trades and zones in first-appearance order) or `trade,days` with `zones`
/// generic zone names (same work in every zone).
pub fn parse(text: &str, zones: &[String]) -> Result<Input, String> {
    let t = Table::parse(text, "takt")?;
    let ct = t.col(&["trade", "wagon", "activity", "crew"]).ok_or("takt CSV needs a 'trade' column")?;
    let cd = t.col(&["days", "duration_days", "duration"]).ok_or("takt CSV needs a 'days' column")?;
    let cz = t.col(&["zone", "location", "area"]);
    let mut inp = Input { trades: vec![], zones: vec![], days: vec![] };
    let num = |v: &str, line: usize| {
        v.trim_end_matches(['d', 'D'])
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|x| x.is_finite() && *x >= 0.0)
            .ok_or(format!("takt line {line}: bad days '{v}'"))
    };
    match cz {
        Some(_) => {
            let mut cells: Vec<(usize, usize, f64)> = vec![];
            for (line, r) in &t.recs {
                let (tr, z) = (get(r, Some(ct)), get(r, cz));
                if tr.is_empty() || z.is_empty() {
                    return Err(format!("takt line {line}: missing trade or zone"));
                }
                let d = num(get(r, Some(cd)), *line)?;
                let i = idx(&mut inp.trades, tr);
                let j = idx(&mut inp.zones, z);
                if cells.iter().any(|c| c.0 == i && c.1 == j) {
                    return Err(format!("takt line {line}: '{tr}' in '{z}' given twice"));
                }
                cells.push((i, j, d));
            }
            inp.days = vec![vec![0.0; inp.zones.len()]; inp.trades.len()];
            for (i, j, d) in cells {
                inp.days[i][j] = d;
            }
        }
        None => {
            if zones.is_empty() {
                return Err("takt CSV has no 'zone' column: give --zones (a count or comma-separated names)".into());
            }
            inp.zones = zones.to_vec();
            for (line, r) in &t.recs {
                let tr = get(r, Some(ct));
                if tr.is_empty() {
                    return Err(format!("takt line {line}: missing trade"));
                }
                if inp.trades.iter().any(|x| x == tr) {
                    return Err(format!("takt line {line}: trade '{tr}' given twice"));
                }
                inp.trades.push(tr.to_string());
                inp.days.push(vec![num(get(r, Some(cd)), *line)?; zones.len()]);
            }
        }
    }
    if inp.trades.is_empty() || inp.zones.is_empty() {
        return Err("takt CSV has no trades or zones".into());
    }
    Ok(inp)
}

fn idx(v: &mut Vec<String>, s: &str) -> usize {
    v.iter().position(|x| x == s).unwrap_or_else(|| {
        v.push(s.to_string());
        v.len() - 1
    })
}

/// `--zones 6` → Z1..Z6; `--zones L1,L2,L3` → those names.
pub fn zone_names(s: &str) -> Result<Vec<String>, String> {
    if let Ok(n) = s.trim().parse::<usize>() {
        if n == 0 || n > 500 {
            return Err("--zones: 1..=500".into());
        }
        return Ok((1..=n).map(|i| format!("Z{i}")).collect());
    }
    let v: Vec<String> = s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
    if v.is_empty() { Err("--zones: give a count or names".into()) } else { Ok(v) }
}

#[derive(Debug, Serialize, Clone)]
pub struct Cell {
    pub trade: String,
    pub zone: String,
    /// Working-day offsets from the start (start inclusive, finish exclusive).
    pub start_day: f64,
    pub finish_day: f64,
    pub days: f64,
    pub start: NaiveDate,
    pub finish: NaiveDate,
    /// Takt mode: work in this zone exceeds the takt time.
    pub overloaded: bool,
}

#[derive(Debug, Serialize)]
pub struct TradeSummary {
    pub trade: String,
    pub total_days: f64,
    pub max_zone_days: f64,
    /// Takt mode: max zone work ÷ takt (above 1 = overloaded). Flow mode: none.
    pub takt_utilization: Option<f64>,
    /// Flow mode: working days per zone (pace).
    pub pace_days_per_zone: f64,
    /// Working days the trade's crew is on site from first start to last finish.
    pub on_site_days: f64,
}

#[derive(Debug, Serialize)]
pub struct TaktPlan {
    pub mode: Mode,
    pub start: NaiveDate,
    pub takt_days: Option<f64>,
    pub buffer: f64,
    pub zones: Vec<String>,
    pub trades: Vec<TradeSummary>,
    pub cells: Vec<Cell>,
    pub total_days: f64,
    pub finish: NaiveDate,
    /// Sum of work ÷ (trades × zones × takt): how full the takt train is (takt mode).
    pub train_load: Option<f64>,
    pub overloaded_cells: usize,
}

/// Working dates from `start` (index 0 = first working day on or after start).
struct Days {
    cal: WorkCalendar,
    v: Vec<NaiveDate>,
}

impl Days {
    fn new(cal: WorkCalendar, start: NaiveDate) -> Days {
        let mut d = start;
        for _ in 0..3700 {
            if cal.is_work(d) {
                break;
            }
            d = d.succ_opt().unwrap_or(d);
        }
        Days { cal, v: vec![d] }
    }
    fn at(&mut self, k: usize) -> NaiveDate {
        while self.v.len() <= k {
            let mut d = *self.v.last().unwrap_or(&NaiveDate::MIN);
            for _ in 0..3700 {
                d = d.succ_opt().unwrap_or(d);
                if self.cal.is_work(d) {
                    break;
                }
            }
            self.v.push(d);
        }
        self.v[k]
    }
    /// Date of a start offset and of a finish offset (exclusive end; a zero-length cell finishes on its start day).
    fn span(&mut self, s: f64, f: f64) -> (NaiveDate, NaiveDate) {
        let a = self.at(s.floor().max(0.0) as usize);
        let b = if f <= s + 1e-9 { a } else { self.at(((f - 1e-9).floor()).max(0.0) as usize) };
        (a, b)
    }
}

#[allow(clippy::needless_range_loop)] // trade × zone grids read clearer with indices
pub fn schedule(inp: &Input, mode: Mode, takt: Option<f64>, buffer: f64, start: NaiveDate, cal: WorkCalendar) -> Result<TaktPlan, String> {
    if !(buffer.is_finite() && buffer >= 0.0) {
        return Err("--buffer must be >= 0".into());
    }
    let (nt, nz) = (inp.trades.len(), inp.zones.len());
    let maxd = inp.days.iter().flatten().fold(0.0f64, |a, b| a.max(*b));
    let mut days = Days::new(cal, start);
    let mut cells = vec![];
    let (mut se, mut fe) = (vec![vec![0.0; nz]; nt], vec![vec![0.0; nz]; nt]);
    let tk = match mode {
        Mode::Takt => {
            let t = takt.unwrap_or(maxd.ceil().max(1.0));
            if !(t.is_finite() && t > 0.0) {
                return Err("--takt must be > 0 days".into());
            }
            for i in 0..nt {
                for j in 0..nz {
                    let s = (i as f64 * (1.0 + buffer) + j as f64) * t;
                    se[i][j] = s;
                    fe[i][j] = s + inp.days[i][j];
                }
            }
            Some(t)
        }
        Mode::Flow => {
            for i in 0..nt {
                // Continuous crew: zone j starts at s0 + sum of earlier zones; s0 is the earliest start that never
                // overtakes trade i-1 (plus buffer days) in any zone.
                let mut s0: f64 = 0.0;
                let mut cum = 0.0;
                for j in 0..nz {
                    if i > 0 {
                        s0 = s0.max(fe[i - 1][j] + buffer - cum);
                    }
                    cum += inp.days[i][j];
                }
                let mut at = s0;
                for j in 0..nz {
                    se[i][j] = at;
                    at += inp.days[i][j];
                    fe[i][j] = at;
                }
            }
            None
        }
    };
    let total = match tk {
        Some(t) => (((nt as f64 - 1.0) * (1.0 + buffer) + nz as f64) * t).max(fe.iter().flatten().fold(0.0f64, |a, b| a.max(*b))),
        None => fe.iter().flatten().fold(0.0f64, |a, b| a.max(*b)),
    };
    if total > 50_000.0 {
        return Err(format!("takt plan is {total:.0} working days long; check the days column and --takt"));
    }
    let mut overloaded = 0;
    for i in 0..nt {
        for j in 0..nz {
            let ov = tk.is_some_and(|t| inp.days[i][j] > t + 1e-9);
            overloaded += ov as usize;
            let (a, b) = days.span(se[i][j], fe[i][j]);
            cells.push(Cell {
                trade: inp.trades[i].clone(),
                zone: inp.zones[j].clone(),
                start_day: se[i][j],
                finish_day: fe[i][j],
                days: inp.days[i][j],
                start: a,
                finish: b,
                overloaded: ov,
            });
        }
    }
    let trades = (0..nt)
        .map(|i| {
            let tot: f64 = inp.days[i].iter().sum();
            let mx = inp.days[i].iter().fold(0.0f64, |a, b| a.max(*b));
            TradeSummary {
                trade: inp.trades[i].clone(),
                total_days: tot,
                max_zone_days: mx,
                takt_utilization: tk.map(|t| mx / t),
                pace_days_per_zone: tot / nz as f64,
                on_site_days: fe[i][nz - 1] - se[i][0],
            }
        })
        .collect();
    let work: f64 = inp.days.iter().flatten().sum();
    let (_, finish) = days.span(0.0, total);
    Ok(TaktPlan {
        mode,
        start: days.at(0),
        takt_days: tk,
        buffer,
        zones: inp.zones.clone(),
        trades,
        cells,
        total_days: total,
        finish,
        train_load: tk.map(|t| work / (nt as f64 * nz as f64 * t)),
        overloaded_cells: overloaded,
    })
}

pub fn render_text(p: &TaktPlan) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    match p.takt_days {
        Some(t) => {
            let _ = writeln!(
                s,
                "Takt plan: {} trades x {} zones, takt {t} wd, {} buffer wagon(s) between trades",
                p.trades.len(),
                p.zones.len(),
                p.buffer
            );
        }
        None => {
            let _ = writeln!(s, "Flowline (continuous crews): {} trades x {} zones, buffer {} wd", p.trades.len(), p.zones.len(), p.buffer);
        }
    }
    let _ = writeln!(s, "Start {} -> finish {} ({:.1} working days)", p.start, p.finish, p.total_days);
    if let Some(l) = p.train_load {
        let _ = writeln!(
            s,
            "Train load {:.0}% (work / capacity of the takt grid); {} overloaded trade-zone cell(s)",
            l * 100.0,
            p.overloaded_cells
        );
    }
    let _ = writeln!(s, "{:<28} {:>8} {:>8} {:>8} {:>9}", "Trade", "Total", "MaxZone", "Pace/z", "OnSite");
    for t in &p.trades {
        let util = t
            .takt_utilization
            .map(|u| format!("  {:.0}% of takt{}", u * 100.0, if u > 1.0 + 1e-9 { " OVERLOADED" } else { "" }))
            .unwrap_or_default();
        let _ = writeln!(
            s,
            "{:<28} {:>8.1} {:>8.1} {:>8.1} {:>9.1}{util}",
            t.trade.chars().take(28).collect::<String>(),
            t.total_days,
            t.max_zone_days,
            t.pace_days_per_zone,
            t.on_site_days
        );
    }
    for c in p.cells.iter().filter(|c| c.overloaded) {
        let _ = writeln!(s, "  OVERLOADED {} in {}: {:.1} wd > takt", c.trade, c.zone, c.days);
    }
    s
}

/// From-csv plan (one row per trade × zone, with activity IDs `T<i>-Z<j>`, links to the same trade's previous zone and
/// the previous trade's same zone, and the planned start as an SNET date) so the takt plan can become MS Project XML.
pub fn to_plan_csv(p: &TaktPlan) -> String {
    let mut w = csv::Writer::from_writer(vec![]);
    let _ = w.write_record(["activity_id", "name", "start", "duration_days", "predecessors", "outline_level", "resources", "notes"]);
    let nz = p.zones.len();
    for (k, c) in p.cells.iter().enumerate() {
        let (i, j) = (k / nz, k % nz);
        let mut preds = vec![];
        if j > 0 {
            preds.push(format!("T{}-Z{}", i + 1, j));
        }
        if i > 0 {
            preds.push(format!("T{}-Z{}", i, j + 1));
        }
        let _ = w.write_record([
            format!("T{}-Z{}", i + 1, j + 1),
            format!("{} - {}", c.trade, c.zone),
            c.start.to_string(),
            format!("{}", c.days),
            preds.join(";"),
            "1".into(),
            c.trade.clone(),
            format!("{} plan, zone {}", if p.takt_days.is_some() { "takt" } else { "flowline" }, c.zone),
        ]);
    }
    String::from_utf8(w.into_inner().unwrap_or_default()).unwrap_or_default()
}

/// Flowline chart (SVG in an HTML page): x = working days with dates, y = zones, one sloped line per trade.
pub fn render_html(p: &TaktPlan, title: &str) -> String {
    let (nz, nt) = (p.zones.len(), p.trades.len());
    let (lm, tm, w, rh) = (110.0, 40.0, 900.0, 34.0);
    let h = rh * nz as f64;
    let sx = w / p.total_days.max(1.0);
    let colors = ["#1f77b4", "#ff7f0e", "#2ca02c", "#d62728", "#9467bd", "#8c564b", "#e377c2", "#7f7f7f", "#bcbd22", "#17becf"];
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" font-family=\"sans-serif\" font-size=\"11\">",
        lm + w + 200.0,
        tm + h + 40.0
    );
    for (j, z) in p.zones.iter().enumerate() {
        let y = tm + h - (j as f64 + 1.0) * rh;
        svg += &format!(
            "<rect x=\"{lm}\" y=\"{y}\" width=\"{w}\" height=\"{rh}\" fill=\"{}\"/>",
            if j % 2 == 0 { "#f6f6f6" } else { "#ffffff" }
        );
        svg += &format!("<text x=\"{}\" y=\"{}\" text-anchor=\"end\">{}</text>", lm - 6.0, y + rh / 2.0 + 4.0, esc(z));
    }
    let step = ((p.total_days / 12.0).ceil()).max(1.0);
    let mut d = 0.0;
    while d <= p.total_days + 1e-9 {
        let x = lm + d * sx;
        let date = p.cells.iter().find(|c| (c.start_day - d).abs() < 1e-9).map(|c| c.start.to_string()).unwrap_or_default();
        svg += &format!("<line x1=\"{x}\" y1=\"{tm}\" x2=\"{x}\" y2=\"{}\" stroke=\"#ddd\"/>", tm + h);
        svg += &format!("<text x=\"{x}\" y=\"{}\" text-anchor=\"middle\">{d}</text>", tm + h + 14.0);
        if !date.is_empty() {
            svg += &format!("<text x=\"{x}\" y=\"{}\" text-anchor=\"middle\" fill=\"#666\">{date}</text>", tm + h + 28.0);
        }
        d += step;
    }
    for i in 0..nt {
        let col = colors[i % colors.len()];
        for j in 0..nz {
            let c = &p.cells[i * nz + j];
            let (x1, x2) = (lm + c.start_day * sx, lm + c.finish_day * sx);
            let (y1, y2) = (tm + h - j as f64 * rh, tm + h - (j as f64 + 1.0) * rh);
            let sw = if c.overloaded { 4 } else { 2 };
            svg += &format!(
                "<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" stroke=\"{col}\" stroke-width=\"{sw}\"><title>{} / {}: {} to {} ({} wd)</title></line>",
                esc(&c.trade),
                esc(&c.zone),
                c.start,
                c.finish,
                c.days
            );
        }
        let ly = tm + 12.0 + i as f64 * 16.0;
        svg += &format!("<rect x=\"{}\" y=\"{}\" width=\"12\" height=\"4\" fill=\"{col}\"/>", lm + w + 16.0, ly - 4.0);
        svg += &format!("<text x=\"{}\" y=\"{ly}\">{}</text>", lm + w + 34.0, esc(&p.trades[i].trade));
    }
    svg += "</svg>";
    let sub = match p.takt_days {
        Some(t) => format!("Takt {t} wd, {} trades, {} zones, {} to {}", nt, nz, p.start, p.finish),
        None => format!("Flowline, {} trades, {} zones, {} to {}", nt, nz, p.start, p.finish),
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{t}</title></head><body style=\"font-family:sans-serif\"><h2>{t}</h2><p>{sub}. Thick lines: work exceeds the takt.</p>{svg}</body></html>\n",
        t = esc(title)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }
    fn cal() -> WorkCalendar {
        crate::calendar::CalendarSpec::default().resolve(2026).unwrap()
    }

    #[test]
    fn takt_formula_and_overload() {
        let inp = parse("trade,days\nFraming,2\nMEP,3\nDrywall,2\n", &zone_names("4").unwrap()).unwrap();
        let p = schedule(&inp, Mode::Takt, Some(3.0), 0.0, d("2026-11-02"), cal()).unwrap();
        // (wagons + zones - 1) x takt = (3 + 4 - 1) x 3 = 18 working days.
        assert_eq!(p.total_days, 18.0);
        assert_eq!(p.finish, d("2026-11-25"));
        assert_eq!(p.overloaded_cells, 0);
        assert_eq!(p.cells[4].start_day, 3.0, "MEP zone 1 starts one takt after framing");
        let p2 = schedule(&inp, Mode::Takt, Some(2.0), 1.0, d("2026-11-02"), cal()).unwrap();
        assert_eq!(p2.overloaded_cells, 4, "MEP 3 d > takt 2 d in every zone");
        assert_eq!(p2.total_days, ((3.0 - 1.0) * 2.0 + 4.0) * 2.0);
        assert!(to_plan_csv(&p).contains("T2-Z2,MEP - Z2"));
        assert!(render_html(&p, "x").contains("<svg"));
    }

    #[test]
    fn flowline_keeps_crews_continuous() {
        let inp = parse("trade,zone,days\nA,L1,2\nA,L2,2\nB,L1,1\nB,L2,4\n", &[]).unwrap();
        let p = schedule(&inp, Mode::Flow, None, 0.0, d("2026-11-02"), cal()).unwrap();
        // B could start L1 at day 2, but its crew would then reach L2 at 3, before A leaves L2 at 4; a continuous crew
        // therefore starts at 3: L1 3..4, L2 4..8.
        assert_eq!((p.cells[2].start_day, p.cells[3].start_day, p.cells[3].finish_day), (3.0, 4.0, 8.0));
        let inp2 = parse("trade,zone,days\nA,L1,1\nA,L2,4\nB,L1,1\nB,L2,1\n", &[]).unwrap();
        let p2 = schedule(&inp2, Mode::Flow, None, 1.0, d("2026-11-02"), cal()).unwrap();
        // B must not overtake A in L2 (A finishes at 5) plus 1 day buffer: L2 starts at 6, so L1 at 5.
        assert_eq!((p2.cells[2].start_day, p2.cells[3].start_day), (5.0, 6.0));
        assert!(parse("trade,days\nA,1\n", &[]).is_err());
        assert!(parse("trade,zone,days\nA,L1,1\nA,L1,2\n", &[]).is_err());
    }
}
