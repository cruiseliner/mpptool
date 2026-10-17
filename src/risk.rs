//! Monte Carlo schedule risk analysis for CSV plans (calendar-aware).
//!
//! Each run samples every leaf task's duration from a PERT-beta distribution over (optimistic, most likely,
//! pessimistic), where most likely is `duration_days` and optimistic/pessimistic come from the CSV columns
//! `optimistic`/`pessimistic` or default to `opt_factor`×M and `pess_factor`×M. The network is re-scheduled with the
//! same links, lags, start constraints and working calendar as `from-csv`. The report gives finish percentiles and,
//! per task, the criticality index: the share of runs in which the task is on the driving path to the project finish.
use crate::calendar::CalendarSpec;
use crate::from_csv::{self, Network, Row};
use chrono::{NaiveDate, NaiveDateTime};
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct RiskOptions {
    pub runs: usize,
    pub seed: u64,
    pub opt_factor: f64,
    pub pess_factor: f64,
}

impl Default for RiskOptions {
    fn default() -> Self {
        RiskOptions { runs: 10_000, seed: 1, opt_factor: 0.8, pess_factor: 1.5 }
    }
}

#[derive(Debug, Serialize)]
pub struct Percentile {
    pub label: String,
    /// Working days (of `hours_per_day`) from the project start to the finish.
    pub working_days: f64,
    pub finish: NaiveDateTime,
}

#[derive(Debug, Serialize)]
pub struct TaskRisk {
    pub id: usize,
    pub name: String,
    pub optimistic_days: f64,
    pub most_likely_days: f64,
    pub pessimistic_days: f64,
    /// Share of runs (0–1) in which the task was on the driving path.
    pub criticality: f64,
    /// Critical in the deterministic schedule.
    pub critical: bool,
}

#[derive(Debug, Serialize)]
pub struct RiskReport {
    pub project_start: NaiveDate,
    pub runs: usize,
    pub seed: u64,
    pub deterministic_working_days: f64,
    pub deterministic_finish: NaiveDateTime,
    /// Probability of finishing on or before the deterministic finish.
    pub p_deterministic: f64,
    pub mean_working_days: f64,
    pub percentiles: Vec<Percentile>,
    /// Leaf tasks, most critical first.
    pub tasks: Vec<TaskRisk>,
}

/// xoshiro256** seeded through splitmix64: small, fast, and reproducible across platforms and versions.
pub struct Rng([u64; 4]);

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        Rng([next(), next(), next(), next()])
    }
    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.0;
        let r = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        r
    }
    /// Uniform in (0, 1).
    pub fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    pub fn normal(&mut self) -> f64 {
        let (u, v) = (self.uniform(), self.uniform());
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
    /// Gamma(shape ≥ 1, scale 1), Marsaglia–Tsang.
    pub fn gamma(&mut self, a: f64) -> f64 {
        if a < 1.0 {
            let u = self.uniform();
            return self.gamma(a + 1.0) * u.powf(1.0 / a);
        }
        let d = a - 1.0 / 3.0;
        let c = 1.0 / (9.0 * d).sqrt();
        loop {
            let x = self.normal();
            let v = (1.0 + c * x).powi(3);
            if v <= 0.0 {
                continue;
            }
            let u = self.uniform();
            if u.ln() < 0.5 * x * x + d - d * v + d * v.ln() {
                return d * v;
            }
        }
    }
    pub fn beta(&mut self, a: f64, b: f64) -> f64 {
        let x = self.gamma(a);
        let y = self.gamma(b);
        x / (x + y)
    }
}

/// PERT-beta sample with mode `m` on `[o, p]` (returns `m` when the range is empty).
pub fn pert(rng: &mut Rng, o: f64, m: f64, p: f64) -> f64 {
    if p <= o {
        return m;
    }
    let m = m.clamp(o, p);
    let a = 1.0 + 4.0 * (m - o) / (p - o);
    let b = 1.0 + 4.0 * (p - m) / (p - o);
    o + rng.beta(a, b) * (p - o)
}

/// Leaves on the driving path to the latest-finishing leaf.
fn driving_path(net: &Network, dur: &[f64], es: &[f64], ef: &[f64], out: &mut Vec<usize>) {
    out.clear();
    let Some(mut i) = net.topo.iter().copied().max_by(|&a, &b| ef[a].total_cmp(&ef[b]).then(b.cmp(&a))) else { return };
    for _ in 0..=net.n {
        out.push(i);
        let next = net.incoming[i]
            .iter()
            .map(|&k| &net.edges[k])
            .find(|(p, _, ty, lag)| (Network::link_value(ty, es[*p], ef[*p], *lag, dur[i]) - es[i]).abs() < 1e-6);
        match next {
            Some(e) => i = e.0,
            None => break,
        }
    }
}

/// Run the simulation over `rows` on calendar `cal`.
pub fn simulate(rows: &[Row], today: NaiveDate, cal: &CalendarSpec, opts: &RiskOptions) -> Result<RiskReport, String> {
    if opts.runs == 0 || opts.runs > 1_000_000 {
        return Err(format!("runs must be 1..=1000000 (got {})", opts.runs));
    }
    if !(opts.opt_factor >= 0.0 && opts.opt_factor <= 1.0 && opts.pess_factor >= 1.0 && opts.pess_factor.is_finite()) {
        return Err("need 0 <= --opt <= 1 <= --pess".into());
    }
    for (i, r) in rows.iter().enumerate() {
        let (o, p) = (r.optimistic.unwrap_or(r.dur * opts.opt_factor), r.pessimistic.unwrap_or(r.dur * opts.pess_factor));
        if o > r.dur + 1e-9 || p + 1e-9 < r.dur {
            return Err(format!("task {} '{}': need optimistic <= duration_days <= pessimistic ({o} / {} / {p})", i + 1, r.name, r.dur));
        }
    }
    let plan = from_csv::schedule_with(rows, None, today, cal)?;
    let mut net = Network::new(rows, today, cal)?;
    let mpd = net.minutes_per_day;
    let n = net.n;
    let leaf = |i: usize| !net.summary[i];
    let ml: Vec<f64> = (0..n).map(|i| if leaf(i) { rows[i].dur } else { 0.0 }).collect();
    let o: Vec<f64> = (0..n).map(|i| rows[i].optimistic.unwrap_or(ml[i] * opts.opt_factor)).collect();
    let p: Vec<f64> = (0..n).map(|i| rows[i].pessimistic.unwrap_or(ml[i] * opts.pess_factor)).collect();

    let det_dur: Vec<f64> = ml.iter().map(|d| d * mpd).collect();
    let (_, def) = net.forward(&det_dur);
    let det = net.topo.iter().map(|&i| def[i]).fold(0.0, f64::max);

    let mut rng = Rng::new(opts.seed);
    let mut finishes = Vec::with_capacity(opts.runs);
    let mut crit = vec![0usize; n];
    let mut dur = vec![0.0; n];
    let mut path = vec![];
    for _ in 0..opts.runs {
        for i in 0..n {
            dur[i] = if leaf(i) { pert(&mut rng, o[i], ml[i], p[i]) * mpd } else { 0.0 };
        }
        let (es, ef) = net.forward(&dur);
        finishes.push(net.topo.iter().map(|&i| ef[i]).fold(0.0, f64::max));
        driving_path(&net, &dur, &es, &ef, &mut path);
        for &i in &path {
            crit[i] += 1;
        }
    }
    finishes.sort_by(f64::total_cmp);
    let runs = opts.runs as f64;
    let q = |x: f64| finishes[((x * runs) as usize).min(finishes.len() - 1)];
    let tl = &mut net.timeline;
    let percentiles = [("P10", 0.1), ("P50", 0.5), ("P80", 0.8), ("P90", 0.9)]
        .iter()
        .map(|&(l, x)| Percentile { label: l.into(), working_days: q(x) / mpd, finish: tl.to_dt(q(x), true) })
        .collect();
    let mut tasks: Vec<TaskRisk> = (0..n)
        .filter(|&i| leaf(i))
        .map(|i| TaskRisk {
            id: i + 1,
            name: rows[i].name.clone(),
            optimistic_days: o[i],
            most_likely_days: ml[i],
            pessimistic_days: p[i],
            criticality: crit[i] as f64 / runs,
            critical: plan.tasks[i].critical,
        })
        .collect();
    tasks.sort_by(|a, b| b.criticality.total_cmp(&a.criticality).then(a.id.cmp(&b.id)));
    Ok(RiskReport {
        project_start: tl.base(),
        runs: opts.runs,
        seed: opts.seed,
        deterministic_working_days: det / mpd,
        deterministic_finish: tl.to_dt(det, true),
        p_deterministic: finishes.iter().filter(|f| **f <= det + 1e-6).count() as f64 / runs,
        mean_working_days: finishes.iter().sum::<f64>() / runs / mpd,
        percentiles,
        tasks,
    })
}

/// Plain-text report.
pub fn render_text(r: &RiskReport) -> String {
    use std::fmt::Write;
    let dt = |d: &NaiveDateTime| d.format("%Y-%m-%d %H:%M").to_string();
    let mut s = String::new();
    let _ = writeln!(
        s,
        "Project start {}; deterministic finish {:.1} wd -> {}",
        r.project_start,
        r.deterministic_working_days,
        dt(&r.deterministic_finish)
    );
    let _ = writeln!(s, "P(finish <= deterministic) = {:.0}%  ({} runs, seed {})", r.p_deterministic * 100.0, r.runs, r.seed);
    let _ = writeln!(s, "Mean: {:.1} wd", r.mean_working_days);
    for p in &r.percentiles {
        let _ = writeln!(s, "{}: {:.1} wd -> {}", p.label, p.working_days, dt(&p.finish));
    }
    let _ = writeln!(s, "Criticality index (share of runs on the driving path):");
    for t in r.tasks.iter().filter(|t| t.criticality > 0.0) {
        let flag = if t.critical { " *" } else { "" };
        let name: String = t.name.chars().take(40).collect();
        let _ = writeln!(s, "  {:>4} {:<40} {:6.1}%{flag}", t.id, name, t.criticality * 100.0);
    }
    let _ = writeln!(s, "  (* = critical in the deterministic schedule)");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::from_csv::parse_rows;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn rng_is_reproducible_and_beta_has_right_mean() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        assert_eq!(a.next_u64(), b.next_u64());
        let mut r = Rng::new(1);
        let n = 20_000;
        let m: f64 = (0..n).map(|_| r.beta(2.0, 5.0)).sum::<f64>() / n as f64;
        assert!((m - 2.0 / 7.0).abs() < 0.01, "beta(2,5) mean {m}");
        // PERT mean = (O + 4M + P) / 6.
        let pm: f64 = (0..n).map(|_| pert(&mut r, 8.0, 10.0, 15.0)).sum::<f64>() / n as f64;
        assert!((pm - (8.0 + 40.0 + 15.0) / 6.0).abs() < 0.05, "pert mean {pm}");
        assert_eq!(pert(&mut r, 3.0, 3.0, 3.0), 3.0);
    }

    const CSV: &str = "name,start,duration_days,predecessors,optimistic,pessimistic\n\
        A,2026-11-02,10,,,\n\
        B,,5,1,5,5\n\
        C,,4,1,2,12\n\
        Done,,0,2;3,,\n";

    #[test]
    fn percentiles_and_criticality() {
        let rows = parse_rows(CSV).unwrap();
        let opts = RiskOptions { runs: 4000, ..Default::default() };
        let r = simulate(&rows, d("2026-01-01"), &CalendarSpec::default(), &opts).unwrap();
        assert_eq!(r.deterministic_working_days, 15.0);
        assert_eq!(r.deterministic_finish.to_string(), "2026-11-20 17:00:00");
        let w: Vec<f64> = r.percentiles.iter().map(|p| p.working_days).collect();
        assert!(w.windows(2).all(|x| x[0] <= x[1]));
        assert!(w[0] >= 8.0 * 1.0 + 5.0 - 1e-9 && w[3] <= 15.0 + 12.0);
        let get = |id| r.tasks.iter().find(|t| t.id == id).unwrap();
        assert_eq!(get(1).criticality, 1.0);
        // C (4d, up to 12d) beats B's fixed 5d only part of the time.
        assert!(get(3).criticality > 0.05 && get(3).criticality < 0.95, "{}", get(3).criticality);
        assert!((get(2).criticality + get(3).criticality - 1.0).abs() < 0.05);
        // Same seed, same answer.
        let r2 = simulate(&rows, d("2026-01-01"), &CalendarSpec::default(), &opts).unwrap();
        assert_eq!(r2.percentiles[1].working_days, r.percentiles[1].working_days);
        assert!(render_text(&r).contains("P80:"));
    }

    #[test]
    fn honours_holidays() {
        let rows = parse_rows("name,start,duration_days\nA,2026-11-23,5\n").unwrap();
        let cal = CalendarSpec { holidays: "US".into(), ..Default::default() };
        let opts = RiskOptions { runs: 10, opt_factor: 1.0, pess_factor: 1.0, ..Default::default() };
        let r = simulate(&rows, d("2026-01-01"), &cal, &opts).unwrap();
        // Thanksgiving (Thu 26th) pushes a 5-day task from Mon 23rd to Mon 30th.
        assert_eq!(r.percentiles[2].finish.to_string(), "2026-11-30 17:00:00");
        assert!(simulate(&rows, d("2026-01-01"), &cal, &RiskOptions { runs: 0, ..Default::default() }).is_err());
        let bad = parse_rows("name,duration_days,optimistic\nA,5,7\n").unwrap();
        assert!(simulate(&bad, d("2026-01-01"), &cal, &opts).is_err());
    }
}

/// Convert a parsed schedule (any format mpptool reads) into CSV-style rows plus its project calendar, so CPM and
/// Monte Carlo run on its task network. Durations are *remaining* durations (`duration × (1 − % complete)`);
/// finished tasks are anchored at their finish date, in-progress tasks at the status date; otherwise a task's start is anchored with its SNET/MSO constraint date, or its actual/current start when it has no
/// predecessors. The project summary (UID 0) and blank rows are skipped.
pub fn rows_from_plan(plan: &crate::model::Plan) -> Result<(Vec<Row>, CalendarSpec), String> {
    use crate::calendar::CustomDay;
    use chrono::Timelike;
    let hpd = if plan.minutes_per_day > 0 { plan.minutes_per_day as f64 / 60.0 } else { 8.0 };
    let tasks: Vec<&crate::model::TaskRec> =
        plan.tasks.iter().filter(|t| t.uid != 0 && !(t.name.trim().is_empty() && t.start.is_none() && t.finish.is_none())).collect();
    if tasks.is_empty() {
        return Err("schedule has no tasks".into());
    }
    let row_of: std::collections::HashMap<i32, usize> = tasks.iter().enumerate().map(|(i, t)| (t.uid, i + 1)).collect();
    let min_level = tasks.iter().map(|t| t.outline_level).min().unwrap_or(1);
    let mut rows = vec![];
    for t in &tasks {
        let preds: Vec<(usize, String, f64)> = t
            .predecessors
            .iter()
            .filter_map(|p| row_of.get(&p.predecessor_uid).map(|&r| (r, p.link_type.to_uppercase(), p.lag_hours / hpd)))
            .filter(|p| ["FS", "SS", "FF", "SF"].contains(&p.1.as_str()))
            .collect();
        let remaining = (1.0 - (t.percent_complete / 100.0).clamp(0.0, 1.0)) * t.duration_hours.max(0.0) / hpd;
        let dated = matches!(t.constraint_type, Some(2) | Some(4)).then_some(t.constraint_date).flatten();
        let pct = t.percent_complete;
        let start = if !t.summary && pct >= 100.0 {
            // Done: a zero-length anchor at its actual finish, so successors cannot start earlier.
            t.actual_finish.or(t.finish).map(|d| d.date())
        } else if !t.summary && pct > 0.0 {
            // In progress: remaining work starts at the status date (or where the file says it is now).
            plan.status_date.or(t.start).map(|d| d.date())
        } else {
            dated.or(if t.predecessors.is_empty() { t.actual_start.or(t.start) } else { None }).map(|d| d.date())
        };
        rows.push(Row {
            name: if t.name.trim().is_empty() { format!("(UID {})", t.uid) } else { t.name.clone() },
            start,
            dur: if t.summary { 0.0 } else { remaining },
            preds,
            level: (t.outline_level - min_level + 1).max(1),
            res: vec![],
            notes: None,
            optimistic: None,
            pessimistic: None,
        });
    }
    // Project calendar.
    let mut spec = CalendarSpec { hours_per_day: hpd, ..Default::default() };
    let cal = plan.default_calendar_uid.and_then(|u| plan.calendars.iter().find(|c| c.uid == u)).or(plan.calendars.first());
    if let Some(c) = cal {
        let m = |t: &chrono::NaiveTime| t.hour() * 60 + t.minute();
        let conv = |r: &[(chrono::NaiveTime, chrono::NaiveTime)]| -> crate::calendar::Ranges {
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
                d = d.succ_opt().ok_or("date overflow")?;
            }
        }
        spec.name = c.name.clone();
    }
    Ok((rows, spec))
}

/// Read a simple JSON task list (the `tasks --json` output of mpptool, or the same shape written by hand) into a plan:
/// an array of objects with `uid`, `name`, `outline_level`, `duration_hours`, optional `start`, `summary`,
/// `percent_complete`, `constraint_type`, `constraint_date` and `predecessors` [{`predecessor_uid`, `link_type`, `lag_hours`}].
pub fn plan_from_json(text: &str) -> Result<crate::model::Plan, String> {
    use crate::model::{Plan, Pred, TaskRec};
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("JSON: {e}"))?;
    let arr = v
        .as_array()
        .or_else(|| v.get("tasks").and_then(|t| t.as_array()))
        .ok_or("JSON must be an array of tasks (or {\"tasks\": [...]})")?;
    let dt = |x: &serde_json::Value| {
        x.as_str().and_then(|s| {
            NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
                .ok()
                .or_else(|| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(8, 0, 0)))
        })
    };
    let mut plan = Plan { format: "JSON".into(), minutes_per_day: 480, minutes_per_week: 2400, ..Default::default() };
    for (i, t) in arr.iter().enumerate() {
        let f = |k: &str| t.get(k).and_then(|x| x.as_f64());
        let uid = t.get("uid").and_then(|x| x.as_i64()).unwrap_or(i as i64 + 1) as i32;
        let preds = t
            .get("predecessors")
            .and_then(|p| p.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        Some(Pred {
                            predecessor_uid: p.get("predecessor_uid")?.as_i64()? as i32,
                            predecessor_id: None,
                            link_type: p.get("link_type").and_then(|x| x.as_str()).unwrap_or("FS").to_string(),
                            lag_hours: p.get("lag_hours").and_then(|x| x.as_f64()).unwrap_or(0.0),
                            lag_display: String::new(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        plan.tasks.push(TaskRec {
            uid,
            id: Some(i as i32 + 1),
            name: t.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            outline_level: t.get("outline_level").and_then(|x| x.as_i64()).unwrap_or(1) as i32,
            start: t.get("start").and_then(dt),
            duration_hours: f("duration_hours").ok_or(format!("JSON task {}: missing duration_hours", i + 1))?,
            percent_complete: f("percent_complete").unwrap_or(0.0),
            summary: t.get("summary").and_then(|x| x.as_bool()).unwrap_or(false),
            constraint_type: t.get("constraint_type").and_then(|x| x.as_i64()).map(|c| c as i32),
            constraint_date: t.get("constraint_date").and_then(dt),
            predecessors: preds,
            ..Default::default()
        });
    }
    Ok(plan)
}

/// Load rows + calendar from any input: `.csv` (from-csv format), `.json` (task list) or any schedule mpptool reads.
/// `csv_cal` applies to CSV input; imported schedules use their own project calendar.
pub fn load_any(
    path: &std::path::Path,
    csv_cal: &CalendarSpec,
    ropts: &crate::ReadOptions,
    engine: crate::Engine,
) -> Result<(Vec<Row>, CalendarSpec), String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let name = |e: String| format!("{}: {e}", path.display());
    match ext.as_str() {
        "csv" => {
            let text = std::fs::read_to_string(path).map_err(|e| name(e.to_string()))?;
            Ok((from_csv::parse_rows(&text).map_err(name)?, csv_cal.clone()))
        }
        "json" => {
            let text = std::fs::read_to_string(path).map_err(|e| name(e.to_string()))?;
            rows_from_plan(&plan_from_json(&text).map_err(name)?).map_err(name)
        }
        _ => rows_from_plan(&crate::read(path, ropts, engine)?).map_err(name),
    }
}

/// One CPM row for `cpm` output.
#[derive(Debug, Serialize)]
pub struct CpmRow {
    pub row: usize,
    pub name: String,
    pub outline_level: i32,
    pub summary: bool,
    pub early_start: Option<NaiveDateTime>,
    pub early_finish: Option<NaiveDateTime>,
    pub remaining_days: f64,
    pub total_float_days: f64,
    pub free_float_days: f64,
    pub critical: bool,
}

/// Critical path (forward/backward pass, total and free float) of rows on `cal`, in working days of `hours_per_day`.
pub fn cpm(rows: &[Row], today: NaiveDate, cal: &CalendarSpec) -> Result<Vec<CpmRow>, String> {
    let p = from_csv::schedule_with(rows, None, today, cal)?;
    let hpd = cal.hours_per_day;
    Ok(p.tasks
        .iter()
        .enumerate()
        .map(|(i, t)| CpmRow {
            row: i + 1,
            name: t.name.clone(),
            outline_level: t.outline_level,
            summary: t.summary,
            early_start: t.start,
            early_finish: t.finish,
            remaining_days: t.duration_hours / hpd,
            total_float_days: t.total_slack_hours.unwrap_or(0.0) / hpd,
            free_float_days: t.free_slack_hours.unwrap_or(0.0) / hpd,
            critical: t.critical,
        })
        .collect())
}
