//! Build a new schedule from a simple CSV and compute dates on a working calendar (default: Microsoft Project's
//! Standard calendar, Mon–Fri 08:00–17:00; see [`crate::calendar`] for holidays, 6/7-day weeks and overtime shifts).
//!
//! Columns (header required, case-insensitive): `name, start, duration_days, predecessors, outline_level [, resources, notes]`
//! - `start`: `YYYY-MM-DD` (optional) — saved as a Start-No-Earlier-Than constraint. The earliest start is the
//!   project start; if no row has a start, the project starts today.
//! - `predecessors`: row numbers (= task IDs, 1-based) separated by `;` or `,` with optional type and lag in
//!   working days, e.g. `2`, `2FS`, `3SS+2`, `4FF-1d`, `2;3`.
//! - `outline_level`: 1 = top level (default). A row followed by deeper rows becomes a summary task.
//! - `resources`: optional, names separated by `;`. `notes`: optional free text.
//! - `optimistic`, `pessimistic`: optional three-point estimates in days, used by the `risk` command.
//! - A duration "day" is `hours_per_day` working hours (default 8), so a 1-day task fills 80% of a 10-hour shift.
//!
//! After the forward pass a backward pass computes total and free slack; tasks with zero total slack are critical.
use crate::calendar::{CalendarSpec, Timeline};
use crate::model::*;
use chrono::{Datelike, NaiveDate};
use std::collections::HashMap;
use std::path::Path;

const EPS: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Row {
    pub name: String,
    pub start: Option<NaiveDate>,
    pub dur: f64,
    pub preds: Vec<(usize, String, f64)>,
    pub level: i32,
    pub res: Vec<String>,
    pub notes: Option<String>,
    /// Optional three-point estimate (days) for `risk`: CSV columns `optimistic`, `pessimistic`.
    pub optimistic: Option<f64>,
    pub pessimistic: Option<f64>,
    /// Uncertainty class for `risk` (CSV column `uncertainty`), e.g. `labor`, `vendor`, `fixed`.
    pub uncertainty: Option<String>,
    /// Progress (from a progress CSV, see [`apply_progress`]).
    pub actual_start: Option<NaiveDate>,
    pub actual_finish: Option<NaiveDate>,
    /// Remaining duration in days for an in-progress task (default: `dur × (1 − percent_complete)`).
    pub remaining: Option<f64>,
    pub percent_complete: Option<f64>,
}

impl Row {
    pub fn is_complete(&self) -> bool {
        self.actual_finish.is_some()
    }
    pub fn in_progress(&self) -> bool {
        self.actual_start.is_some() && self.actual_finish.is_none()
    }
    /// Work left in days: 0 when complete, the remaining duration when in progress, else the duration.
    pub fn remaining_days(&self) -> f64 {
        if self.is_complete() {
            0.0
        } else if self.in_progress() {
            self.remaining.unwrap_or(self.dur * (1.0 - self.percent_complete.unwrap_or(0.0).clamp(0.0, 100.0) / 100.0)).max(0.0)
        } else {
            self.dur
        }
    }
}

/// Status-date options for scheduling an update.
#[derive(Debug, Clone, Default)]
pub struct ScheduleOptions {
    /// Data date: remaining work is scheduled after this day. Required when any row has progress.
    pub status_date: Option<NaiveDate>,
    /// Delay tasks within their total float to remove resource over-allocation (each named resource = 1 unit).
    pub level: bool,
}

/// What the scheduler did beyond plain CPM.
#[derive(Debug, Clone, Default)]
pub struct ScheduleInfo {
    /// Row numbers (1-based) delayed by leveling.
    pub leveled: Vec<usize>,
    /// Row numbers still over-allocated because they had no float to absorb the delay.
    pub unresolved: Vec<usize>,
}

/// Apply a progress CSV to `rows`. Columns (header, case-insensitive): `id` (row number) or `name`, and any of
/// `actual_start`, `actual_finish` (YYYY-MM-DD), `remaining_days`, `percent_complete` (0–100).
/// `percent_complete` 100 without an actual finish is an error, as is an actual finish without an actual start.
pub fn apply_progress(rows: &mut [Row], text: &str) -> Result<usize, String> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).trim(csv::Trim::All).comment(Some(b'#')).from_reader(text.as_bytes());
    let hdr: Vec<String> = rdr.headers().map_err(|e| format!("progress CSV header: {e}"))?.iter().map(|h| h.to_lowercase()).collect();
    let col = |n: &str| hdr.iter().position(|h| h == n);
    let (ci, cn) = (col("id").or(col("row")), col("name"));
    if ci.is_none() && cn.is_none() {
        return Err("progress CSV needs an 'id' (row number) or 'name' column".into());
    }
    let (cas, caf, crem, cpct) =
        (col("actual_start"), col("actual_finish"), col("remaining_days").or(col("remaining")), col("percent_complete"));
    let mut count = 0;
    for (k, rec) in rdr.records().enumerate() {
        let line = k + 2;
        let rec = rec.map_err(|e| format!("progress line {line}: {e}"))?;
        let g = |c: Option<usize>| c.and_then(|c| rec.get(c)).unwrap_or("").trim().to_string();
        let idx = match g(ci).as_str() {
            "" => {
                let nm = g(cn);
                if nm.is_empty() {
                    continue;
                }
                let hits: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| r.name.eq_ignore_ascii_case(&nm)).map(|(i, _)| i).collect();
                match hits.as_slice() {
                    [i] => *i,
                    [] => return Err(format!("progress line {line}: no task named '{nm}'")),
                    _ => return Err(format!("progress line {line}: several tasks named '{nm}'; use the id column")),
                }
            }
            v => v
                .parse::<usize>()
                .ok()
                .filter(|i| *i >= 1 && *i <= rows.len())
                .map(|i| i - 1)
                .ok_or(format!("progress line {line}: bad id '{v}'"))?,
        };
        let date = |c: Option<usize>, what: &str| -> Result<Option<NaiveDate>, String> {
            match g(c).as_str() {
                "" => Ok(None),
                v => NaiveDate::parse_from_str(v, "%Y-%m-%d").map(Some).map_err(|_| format!("progress line {line}: bad {what} '{v}'")),
            }
        };
        let num = |c: Option<usize>, what: &str| -> Result<Option<f64>, String> {
            match g(c).as_str() {
                "" => Ok(None),
                v => v
                    .trim_end_matches(['%', 'd', 'D'])
                    .parse::<f64>()
                    .ok()
                    .filter(|x| *x >= 0.0)
                    .map(Some)
                    .ok_or(format!("progress line {line}: bad {what} '{v}'")),
            }
        };
        let r = &mut rows[idx];
        r.actual_start = date(cas, "actual_start")?.or(r.actual_start);
        r.actual_finish = date(caf, "actual_finish")?.or(r.actual_finish);
        r.remaining = num(crem, "remaining_days")?.or(r.remaining);
        r.percent_complete = num(cpct, "percent_complete")?.map(|p| p.min(100.0)).or(r.percent_complete);
        if r.actual_finish.is_some() && r.actual_start.is_none() {
            return Err(format!("progress line {line}: '{}' has an actual finish but no actual start", r.name));
        }
        if let (Some(a), Some(f)) = (r.actual_start, r.actual_finish)
            && f < a
        {
            return Err(format!("progress line {line}: '{}' finishes before it starts", r.name));
        }
        if r.percent_complete == Some(100.0) && r.actual_finish.is_none() {
            return Err(format!("progress line {line}: '{}' is 100% complete but has no actual_finish", r.name));
        }
        count += 1;
    }
    Ok(count)
}

/// Parse one predecessor token such as `3`, `3SS`, `4FF-1d`, `2FS+2days`.
pub fn parse_pred(s: &str) -> Result<(usize, String, f64), String> {
    let s = s.trim().to_uppercase();
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    let id: usize = digits.parse().map_err(|_| format!("bad predecessor '{s}'"))?;
    let mut rest = &s[digits.len()..];
    let mut ty = "FS".to_string();
    for t in ["FS", "SS", "FF", "SF"] {
        if let Some(r) = rest.strip_prefix(t) {
            ty = t.into();
            rest = r;
            break;
        }
    }
    let rest = rest.trim();
    let rest = rest.strip_suffix("DAYS").or(rest.strip_suffix("DAY")).or(rest.strip_suffix('D')).unwrap_or(rest).trim();
    let lag = if rest.is_empty() {
        0.0
    } else {
        rest.trim_start_matches('+').trim().parse::<f64>().map_err(|_| format!("bad lag in predecessor '{s}'"))?
    };
    Ok((id, ty, lag))
}

/// Parse CSV text into rows.
pub fn parse_rows(text: &str) -> Result<Vec<Row>, String> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).trim(csv::Trim::All).from_reader(text.as_bytes());
    let hdr: Vec<String> = rdr.headers().map_err(|e| format!("CSV header: {e}"))?.iter().map(|h| h.to_lowercase()).collect();
    let col = |n: &str| hdr.iter().position(|h| h == n);
    let ci = col("name").ok_or("CSV needs a 'name' column")?;
    let (cs, cd, cp, cl, cr, cn) =
        (col("start"), col("duration_days").or(col("duration")), col("predecessors"), col("outline_level"), col("resources"), col("notes"));
    let cu = col("uncertainty").or(col("class"));
    let (co, cpe) = (col("optimistic").or(col("optimistic_days")), col("pessimistic").or(col("pessimistic_days")));
    let mut rows = vec![];
    for (n, rec) in rdr.records().enumerate() {
        let line = n + 2;
        let rec = rec.map_err(|e| format!("CSV line {line}: {e}"))?;
        let g = |c: Option<usize>| c.and_then(|c| rec.get(c)).unwrap_or("").trim().to_string();
        let name = g(Some(ci));
        if name.is_empty() {
            continue;
        }
        let start = match g(cs).as_str() {
            "" => None,
            s => {
                Some(NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| format!("line {line}: bad start '{s}' (want YYYY-MM-DD): {e}"))?)
            }
        };
        let dur = match g(cd).as_str() {
            "" => 1.0,
            s => s
                .trim_end_matches(['d', 'D'])
                .parse::<f64>()
                .ok()
                .filter(|v| *v >= 0.0)
                .ok_or(format!("line {line}: bad duration '{s}'"))?,
        };
        let preds = g(cp)
            .split([';', ','])
            .filter(|s| !s.trim().is_empty())
            .map(parse_pred)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("line {line}: {e}"))?;
        let level = match g(cl).as_str() {
            "" => 1,
            s => s.parse().ok().filter(|v| *v >= 1).ok_or(format!("line {line}: bad outline_level '{s}'"))?,
        };
        let res = g(cr).split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        let notes = Some(g(cn)).filter(|s| !s.is_empty());
        let est = |c: Option<usize>, what: &str| -> Result<Option<f64>, String> {
            match g(c).as_str() {
                "" => Ok(None),
                s => s
                    .trim_end_matches(['d', 'D'])
                    .parse::<f64>()
                    .ok()
                    .filter(|v| *v >= 0.0)
                    .map(Some)
                    .ok_or(format!("line {line}: bad {what} '{s}'")),
            }
        };
        let (optimistic, pessimistic) = (est(co, "optimistic")?, est(cpe, "pessimistic")?);
        let uncertainty = Some(g(cu).to_lowercase()).filter(|s| !s.is_empty());
        rows.push(Row { name, start, dur, preds, level, res, notes, optimistic, pessimistic, uncertainty, ..Default::default() });
    }
    if rows.is_empty() {
        return Err("CSV has no tasks".into());
    }
    Ok(rows)
}

/// Read a CSV file and schedule it on the Standard calendar.
pub fn build(path: &Path) -> Result<Plan, String> {
    build_with(path, &CalendarSpec::default())
}

/// Read a CSV file and schedule it on calendar `cal`.
pub fn build_with(path: &Path, cal: &CalendarSpec) -> Result<Plan, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string());
    schedule_with(&parse_rows(&text).map_err(|e| format!("{}: {e}", path.display()))?, name, chrono::Local::now().date_naive(), cal)
}

/// The dependency network of a CSV plan on a working-minute axis, shared by [`schedule_with`] and the
/// Monte Carlo `risk` command.
pub struct Network {
    pub n: usize,
    pub summary: Vec<bool>,
    /// Leaf-to-leaf edges `(pred, succ, type, lag_minutes)`; links on summaries are expanded to their leaves.
    pub edges: Vec<(usize, usize, String, f64)>,
    /// Start-no-earlier-than position of each row, in working minutes from the project start.
    pub snet: Vec<f64>,
    /// Leaves in topological order.
    pub topo: Vec<usize>,
    /// Incoming and outgoing edge indices per row.
    pub incoming: Vec<Vec<usize>>,
    pub outgoing: Vec<Vec<usize>>,
    pub timeline: Timeline,
    pub minutes_per_day: f64,
    /// Working-minute position where remaining work may start (just after the status date).
    pub status_pos: Option<f64>,
    /// Actual start (and, when complete, actual finish) positions per row.
    pub fixed: Vec<Option<(f64, Option<f64>)>>,
}

impl Network {
    /// Build the network (validates references and detects cycles).
    pub fn new(rows: &[Row], today: NaiveDate, cal: &CalendarSpec) -> Result<Network, String> {
        Self::with_status(rows, today, cal, None)
    }

    /// Build the network for a status update with data date `status`.
    pub fn with_status(rows: &[Row], today: NaiveDate, cal: &CalendarSpec, status: Option<NaiveDate>) -> Result<Network, String> {
        let has_progress = rows.iter().any(|r| r.actual_start.is_some());
        if has_progress && status.is_none() {
            return Err("progress given but no status date (use --status-date)".into());
        }
        if let Some(sd) = status
            && let Some(r) = rows.iter().find(|r| r.actual_start.is_some_and(|d| d > sd) || r.actual_finish.is_some_and(|d| d > sd))
        {
            return Err(format!("task '{}': actual dates after the status date {sd}", r.name));
        }
        let n = rows.len();
        for (i, r) in rows.iter().enumerate() {
            for (p, _, _) in &r.preds {
                if *p == 0 || *p > n || *p == i + 1 {
                    return Err(format!("task {} '{}': predecessor {} out of range", i + 1, r.name, p));
                }
            }
        }
        let horizon: f64 =
            rows.iter().map(|r| r.dur.max(r.pessimistic.unwrap_or(0.0)) + r.preds.iter().map(|p| p.2.abs()).sum::<f64>()).sum();
        if !horizon.is_finite() || horizon > 200_000.0 {
            return Err(format!("plan spans {horizon:.0} working days; durations and lags look wrong"));
        }
        let summary: Vec<bool> = (0..n).map(|i| i + 1 < n && rows[i + 1].level > rows[i].level).collect();
        let mut parent = vec![None; n];
        let mut stack: Vec<usize> = vec![];
        for i in 0..n {
            while let Some(&top) = stack.last() {
                if rows[top].level >= rows[i].level {
                    stack.pop();
                } else {
                    break;
                }
            }
            parent[i] = stack.last().copied();
            stack.push(i);
        }
        let leaves_of = |i: usize| -> Vec<usize> {
            if summary[i] { (i + 1..n).take_while(|&k| rows[k].level > rows[i].level).filter(|&k| !summary[k]).collect() } else { vec![i] }
        };
        let mpd = cal.minutes_per_day();
        // A link on a summary applies to all its leaves (as predecessor or successor); links inherited from ancestors too.
        let mut edges: Vec<(usize, usize, String, f64)> = vec![];
        for i in (0..n).filter(|&i| !summary[i]) {
            let mut chain = vec![i];
            let mut c = parent[i];
            while let Some(p) = c {
                chain.push(p);
                c = parent[p];
            }
            for &k in &chain {
                for (p, ty, lag) in &rows[k].preds {
                    for pl in leaves_of(p - 1) {
                        if pl == i {
                            // A link between a task and its own summary (allowed in imported files) adds no constraint.
                            continue;
                        }
                        edges.push((pl, i, ty.clone(), lag * mpd));
                    }
                }
            }
        }
        // Kahn's algorithm over leaves.
        let mut indeg = vec![0usize; n];
        let mut incoming = vec![vec![]; n];
        let mut outgoing = vec![vec![]; n];
        for (k, e) in edges.iter().enumerate() {
            indeg[e.1] += 1;
            incoming[e.1].push(k);
            outgoing[e.0].push(k);
        }
        let mut queue: std::collections::VecDeque<usize> = (0..n).filter(|&i| !summary[i] && indeg[i] == 0).collect();
        let mut topo = vec![];
        while let Some(i) = queue.pop_front() {
            topo.push(i);
            for &k in &outgoing[i] {
                let s = edges[k].1;
                indeg[s] -= 1;
                if indeg[s] == 0 {
                    queue.push_back(s);
                }
            }
        }
        if topo.len() != summary.iter().filter(|s| !**s).count() {
            return Err("could not resolve schedule (circular dependencies?)".into());
        }
        let first = rows.iter().filter_map(|r| r.start).chain(rows.iter().filter_map(|r| r.actual_start)).min().unwrap_or(today);
        let wc = cal.resolve(first.year())?;
        let mut timeline = Timeline::new(wc, first);
        let snet = rows.iter().map(|r| r.start.map(|d| timeline.pos_of_date(d)).unwrap_or(0.0)).collect();
        let status_pos = status.and_then(|d| d.succ_opt()).map(|d| timeline.pos_of_date(d));
        let mut fixed = vec![None; n];
        for (i, r) in rows.iter().enumerate() {
            if let (false, Some(a)) = (summary[i], r.actual_start) {
                let f = r.actual_finish.and_then(|f| f.succ_opt()).map(|f| timeline.pos_of_date(f));
                fixed[i] = Some((timeline.pos_of_date(a), f));
            }
        }
        Ok(Network { n, summary, edges, snet, topo, incoming, outgoing, timeline, minutes_per_day: mpd, status_pos, fixed })
    }

    /// Earliest start of leaf `s` given predecessor dates and its own duration `d` (all in minutes).
    #[inline]
    pub fn link_value(ty: &str, es_p: f64, ef_p: f64, lag: f64, d: f64) -> f64 {
        match ty {
            "SS" => es_p + lag,
            "FF" => ef_p + lag - d,
            "SF" => es_p + lag - d,
            _ => ef_p + lag,
        }
    }

    /// Forward pass with durations `dur` (minutes, per row; summaries ignored). Returns `(es, ef)` per row.
    pub fn forward(&self, dur: &[f64]) -> (Vec<f64>, Vec<f64>) {
        self.forward_ext(dur, &[])
    }

    /// Forward pass with extra start-no-earlier-than positions (`extra[i]`, e.g. from leveling). `dur` holds
    /// remaining durations; complete tasks keep their actual dates and in-progress tasks finish their remaining
    /// work after the status date.
    pub fn forward_ext(&self, dur: &[f64], extra: &[f64]) -> (Vec<f64>, Vec<f64>) {
        let (mut es, mut ef) = (self.snet.clone(), vec![0.0; self.n]);
        let sp = self.status_pos.unwrap_or(0.0);
        for &s in &self.topo {
            match self.fixed[s] {
                Some((a, Some(f))) => {
                    es[s] = a;
                    ef[s] = f.max(a);
                    continue;
                }
                Some((a, None)) => {
                    es[s] = a;
                    ef[s] = a.max(sp) + dur[s];
                    continue;
                }
                None => {}
            }
            let mut v = self.snet[s].max(sp).max(extra.get(s).copied().unwrap_or(0.0));
            for &k in &self.incoming[s] {
                let (p, _, ref ty, lag) = self.edges[k];
                v = v.max(Self::link_value(ty, es[p], ef[p], lag, dur[s]));
            }
            es[s] = v;
            ef[s] = v + dur[s];
        }
        (es, ef)
    }
}

/// Schedule rows on the Standard calendar (Mon–Fri 08:00–17:00, no holidays).
pub fn schedule(rows: &[Row], name: Option<String>, today: NaiveDate) -> Result<Plan, String> {
    schedule_with(rows, name, today, &CalendarSpec::default())
}

/// Schedule rows into a plan on calendar `cal`: forward pass honouring working time, summary roll-up, backward
/// pass for total slack and critical path, and free slack.
pub fn schedule_with(rows: &[Row], name: Option<String>, today: NaiveDate, cal: &CalendarSpec) -> Result<Plan, String> {
    schedule_full(rows, name, today, cal, &ScheduleOptions::default()).map(|r| r.0)
}

/// Backward pass: late finish and free slack per row (leaves only), given early dates.
fn backward(net: &Network, span: &[f64], es: &[f64], ef: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let n = net.n;
    let pfinish = net.topo.iter().map(|&i| ef[i]).fold(0.0, f64::max);
    let mut lf = vec![pfinish; n];
    let mut free = vec![f64::MAX; n];
    for &p in net.topo.iter().rev() {
        for &k in &net.outgoing[p] {
            let (_, s, ref ty, lag) = net.edges[k];
            let d = span[p];
            let ls_s = lf[s] - span[s];
            let v = match ty.as_str() {
                "SS" => ls_s - lag + d,
                "FF" => lf[s] - lag,
                "SF" => lf[s] - lag + d,
                _ => ls_s - lag,
            };
            lf[p] = lf[p].min(v);
            // Free slack: how far p can slip before it moves this successor's early dates.
            let room = match ty.as_str() {
                "SS" => es[s] - lag - es[p],
                "FF" => ef[s] - lag - ef[p],
                "SF" => ef[s] - lag - es[p],
                _ => es[s] - lag - ef[p],
            };
            free[p] = free[p].min(room);
        }
        if free[p] == f64::MAX {
            free[p] = pfinish - ef[p];
        }
    }
    (lf, free)
}

/// Serial resource leveling within total float: tasks are taken in early-start order; a task whose resources
/// are busy is delayed to the first free slot, but only while that keeps it within its late start. Each named
/// resource has a capacity of one task at a time.
fn level(net: &Network, rows: &[Row], dur: &[f64], extra: &mut [f64], info: &mut ScheduleInfo) {
    let n = net.n;
    let mut seen_unresolved = std::collections::BTreeSet::new();
    for _ in 0..(n * 4 + 10) {
        let (es, ef) = net.forward_ext(dur, extra);
        let span: Vec<f64> = (0..n).map(|i| ef[i] - es[i]).collect();
        let (lf, _) = backward(net, &span, &es, &ef);
        let mut order: Vec<usize> = net.topo.iter().copied().filter(|&i| !rows[i].res.is_empty() && span[i] > EPS).collect();
        order.sort_by(|&a, &b| es[a].total_cmp(&es[b]).then((lf[a] - ef[a]).total_cmp(&(lf[b] - ef[b]))).then(a.cmp(&b)));
        let mut booked: HashMap<&str, Vec<(f64, f64)>> = HashMap::new();
        let mut moved = false;
        for &i in &order {
            let d = span[i];
            let mut t = es[i];
            if net.fixed[i].is_none() {
                loop {
                    let clash = rows[i]
                        .res
                        .iter()
                        .filter_map(|r| booked.get(r.as_str()))
                        .flatten()
                        .filter(|(a, b)| *a < t + d - EPS && *b > t + EPS)
                        .map(|(_, b)| *b)
                        .fold(f64::MIN, f64::max);
                    if clash == f64::MIN {
                        break;
                    }
                    t = clash;
                }
            }
            if t > es[i] + EPS {
                if t <= lf[i] - d + EPS {
                    extra[i] = t;
                    if !info.leveled.contains(&(i + 1)) {
                        info.leveled.push(i + 1);
                    }
                    moved = true;
                    break;
                }
                seen_unresolved.insert(i + 1);
                t = es[i];
            }
            for r in &rows[i].res {
                booked.entry(r.as_str()).or_default().push((t, t + d));
            }
        }
        if !moved {
            info.unresolved = seen_unresolved.into_iter().collect();
            info.leveled.sort();
            return;
        }
        seen_unresolved.clear();
    }
}

/// Schedule with status-date progress and optional leveling. Returns the plan and what leveling did.
pub fn schedule_full(
    rows: &[Row],
    name: Option<String>,
    today: NaiveDate,
    cal: &CalendarSpec,
    opts: &ScheduleOptions,
) -> Result<(Plan, ScheduleInfo), String> {
    let mut net = Network::with_status(rows, today, cal, opts.status_date)?;
    let n = net.n;
    let summary = net.summary.clone();
    let mpd = net.minutes_per_day;
    let dur: Vec<f64> = (0..n).map(|i| if summary[i] { 0.0 } else { rows[i].remaining_days() * mpd }).collect();
    let mut extra = vec![0.0; n];
    let mut info = ScheduleInfo::default();
    if opts.level {
        level(&net, rows, &dur, &mut extra, &mut info);
    }
    let (mut es, mut ef) = net.forward_ext(&dur, &extra);
    let span: Vec<f64> = (0..n).map(|i| ef[i] - es[i]).collect();
    let (lf, mut free) = backward(&net, &span, &es, &ef);
    let children = |i: usize| (i + 1..n).take_while(move |&k| rows[k].level > rows[i].level);
    let mut slack = vec![0.0; n];
    let mut critical = vec![false; n];
    for i in (0..n).filter(|&i| !summary[i]) {
        if rows[i].is_complete() {
            free[i] = 0.0;
            continue;
        }
        slack[i] = (lf[i] - ef[i]).max(0.0);
        free[i] = free[i].max(0.0).min(slack[i]);
        critical[i] = slack[i] < EPS;
    }
    for i in (0..n).rev().filter(|&i| summary[i]) {
        let k: Vec<usize> = children(i).collect();
        es[i] = k.iter().map(|&k| es[k]).fold(f64::MAX, f64::min);
        ef[i] = k.iter().map(|&k| ef[k]).fold(f64::MIN, f64::max);
        critical[i] = k.iter().any(|&k| critical[k]);
        slack[i] = k.iter().map(|&k| slack[k]).fold(f64::MAX, f64::min);
        free[i] = k.iter().map(|&k| free[k]).fold(f64::MAX, f64::min);
    }

    let tl = &mut net.timeline;
    let base = tl.base();
    let (cal_name, minutes_per_week) = if cal.is_standard() {
        ("Standard".to_string(), 2400)
    } else {
        let days = cal.week.iter().filter(|r| !r.is_empty()).count() as f64;
        (cal.name.clone(), (cal.minutes_per_day() * days).round() as i32)
    };
    let mut plan = Plan {
        format: "CSV".into(),
        name: name.clone(),
        title: name,
        minutes_per_day: mpd.round() as i32,
        minutes_per_week,
        default_calendar_uid: Some(1),
        calendars: vec![CalendarRec::standard(1)],
        ..Default::default()
    };
    let mut res_uid: HashMap<String, i32> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        let ms = !summary[i] && r.dur == 0.0;
        let start = tl.to_dt(es[i], ms && es[i] > 0.0);
        let finish = if ms { start } else { tl.to_dt(ef[i], true) };
        let h = |m: f64| m / 60.0;
        plan.tasks.push(TaskRec {
            uid: (i + 1) as i32,
            id: Some((i + 1) as i32),
            name: r.name.clone(),
            outline_level: r.level,
            start: Some(start),
            finish: Some(finish),
            duration_hours: h(ef[i] - es[i]),
            milestone: ms,
            summary: summary[i],
            critical: critical[i],
            total_slack_hours: Some(h(slack[i])),
            free_slack_hours: Some(h(free[i])),
            notes: if info.leveled.contains(&(i + 1)) {
                Some(r.notes.clone().map(|n| format!("{n} [leveled by mpptool]")).unwrap_or_else(|| "[leveled by mpptool]".into()))
            } else {
                r.notes.clone()
            },
            constraint_type: if extra[i] > 0.0 { Some(4) } else { r.start.map(|_| 4) },
            constraint_date: if extra[i] > 0.0 { Some(tl.to_dt(extra[i], false)) } else { r.start.map(|d| tl.first_work_at(d)) },
            actual_start: (!summary[i]).then_some(r.actual_start.map(|_| start)).flatten(),
            actual_finish: (!summary[i] && r.is_complete()).then_some(finish),
            percent_complete: if summary[i] {
                0.0
            } else if r.is_complete() {
                100.0
            } else if r.in_progress() {
                r.percent_complete
                    .unwrap_or_else(|| if r.dur > 0.0 { (1.0 - r.remaining_days() / r.dur).clamp(0.0, 0.99) * 100.0 } else { 0.0 })
            } else {
                0.0
            },
            predecessors: r
                .preds
                .iter()
                .map(|(p, ty, lag)| Pred {
                    predecessor_uid: *p as i32,
                    predecessor_id: Some(*p as i32),
                    link_type: ty.clone(),
                    lag_hours: lag * mpd / 60.0,
                    lag_display: format!("{lag}d"),
                })
                .collect(),
            ..Default::default()
        });
        if !summary[i] {
            for rn in &r.res {
                let next = res_uid.len() as i32 + 1;
                let uid = *res_uid.entry(rn.clone()).or_insert_with(|| {
                    plan.resources.push(ResourceRec {
                        uid: next,
                        id: Some(next),
                        name: rn.clone(),
                        rtype: "Work".into(),
                        max_units: Some(100.0),
                        ..Default::default()
                    });
                    next
                });
                let auid = plan.assignments.len() as i32 + 1;
                plan.assignments.push(AssignmentRec {
                    uid: auid,
                    task_uid: (i + 1) as i32,
                    resource_uid: uid,
                    units: 100.0,
                    work_hours: r.dur * mpd / 60.0,
                    start: Some(start),
                    finish: Some(finish),
                    cost: None,
                });
            }
        }
    }
    plan.status_date = opts.status_date.and_then(|d| d.and_hms_opt(17, 0, 0));
    plan.start = plan.tasks.iter().filter_map(|t| t.start).min();
    plan.finish = plan.tasks.iter().filter_map(|t| t.finish).max();
    if !cal.is_standard() {
        let (from, to) = match cal.years {
            Some((a, b)) => (NaiveDate::from_ymd_opt(a, 1, 1), NaiveDate::from_ymd_opt(b, 12, 31)),
            None => {
                let end = plan.finish.map(|f| f.date()).unwrap_or(base);
                (NaiveDate::from_ymd_opt(base.year(), 1, 1), NaiveDate::from_ymd_opt(end.year(), 12, 31))
            }
        };
        plan.calendars = vec![tl.calendar().to_rec(1, &cal_name, from.unwrap_or(base), to.unwrap_or(base))];
    }
    Ok((plan, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn pred_tokens() {
        assert_eq!(parse_pred("3").unwrap(), (3, "FS".into(), 0.0));
        assert_eq!(parse_pred("3ss+2").unwrap(), (3, "SS".into(), 2.0));
        assert_eq!(parse_pred("4FF-1d").unwrap(), (4, "FF".into(), -1.0));
        assert_eq!(parse_pred("2FS+2days").unwrap(), (2, "FS".into(), 2.0));
        assert!(parse_pred("x").is_err());
    }

    #[test]
    fn weekend_skipping() {
        // 2026-10-16 is a Friday.
        let cal = CalendarSpec::default().resolve(2026).unwrap();
        let mut t = Timeline::new(cal, d("2026-10-16"));
        assert_eq!(t.to_dt(480.0, false).date(), d("2026-10-19"));
        assert_eq!(t.pos_of_date(d("2026-10-17")), 480.0); // Saturday rolls to Monday
        assert_eq!(t.to_dt(480.0, true).to_string(), "2026-10-16 17:00:00");
        assert_eq!(t.to_dt(240.0, false).to_string(), "2026-10-16 12:00:00");
        assert_eq!(t.to_dt(360.0, false).to_string(), "2026-10-16 15:00:00");
    }

    const CSV: &str = "name,start,duration_days,predecessors,outline_level\n\
        Phase,,,,1\n\
        A,2026-10-19,3,,2\n\
        B,,2,2,2\n\
        C,,1,2,2\n\
        Done,,0,3;4,1\n";

    #[test]
    fn schedules_and_finds_critical_path() {
        let p = schedule(&parse_rows(CSV).unwrap(), None, d("2026-01-01")).unwrap();
        let t = &p.tasks;
        assert!(t[0].summary && !t[1].summary);
        assert_eq!(fmt_dt(&t[1].start), "2026-10-19 08:00");
        assert_eq!(fmt_dt(&t[1].finish), "2026-10-21 17:00");
        assert_eq!(fmt_dt(&t[2].finish), "2026-10-23 17:00");
        assert!(t[1].critical && t[2].critical && !t[3].critical && t[4].critical);
        assert_eq!(t[3].total_slack_hours, Some(8.0));
        assert!(t[4].milestone);
        assert_eq!(fmt_dt(&t[4].start), "2026-10-23 17:00");
        assert_eq!(t[1].constraint_type, Some(4));
        assert_eq!(fmt_dt(&t[1].constraint_date), "2026-10-19 08:00");
    }

    #[test]
    fn rejects_cycles_and_bad_refs() {
        let cyc = "name,duration_days,predecessors\nA,1,2\nB,1,1\n";
        assert!(schedule(&parse_rows(cyc).unwrap(), None, d("2026-01-01")).unwrap_err().contains("circular"));
        let bad = "name,duration_days,predecessors\nA,1,5\n";
        assert!(schedule(&parse_rows(bad).unwrap(), None, d("2026-01-01")).is_err());
        assert!(parse_rows("name,start\nA,10/19/2026\n").is_err());
    }

    #[test]
    fn progress_update_schedules_remaining_work_after_status_date() {
        let mut rows = parse_rows("name,start,duration_days,predecessors\nA,2026-10-19,3,\nB,,4,1\nC,,2,2\n").unwrap();
        let n = apply_progress(
            &mut rows,
            "id,actual_start,actual_finish,remaining_days,percent_complete\n1,2026-10-19,2026-10-22,,100\n2,2026-10-23,,3,\n",
        )
        .unwrap();
        assert_eq!(n, 2);
        let o = ScheduleOptions { status_date: Some(d("2026-10-26")), level: false };
        let (p, _) = schedule_full(&rows, None, d("2026-01-01"), &CalendarSpec::default(), &o).unwrap();
        let t = &p.tasks;
        assert_eq!(fmt_dt(&t[0].actual_finish), "2026-10-22 17:00"); // finished a day late
        assert_eq!(t[0].percent_complete, 100.0);
        assert_eq!(fmt_dt(&t[1].actual_start), "2026-10-23 08:00");
        assert_eq!(fmt_dt(&t[1].finish), "2026-10-29 17:00"); // 3 days left after Mon 26th
        assert_eq!(fmt_dt(&t[2].start), "2026-10-30 08:00");
        assert!(!t[0].critical && t[1].critical);
        assert_eq!(fmt_dt(&p.status_date), "2026-10-26 17:00");
        // Unstarted work never starts before the status date.
        let rows2 = parse_rows("name,start,duration_days\nX,2026-10-19,2\n").unwrap();
        let (p2, _) = schedule_full(&rows2, None, d("2026-01-01"), &CalendarSpec::default(), &o).unwrap();
        assert_eq!(fmt_dt(&p2.tasks[0].start), "2026-10-27 08:00");
        // Errors.
        let o2 = ScheduleOptions::default();
        assert!(schedule_full(&rows, None, d("2026-01-01"), &CalendarSpec::default(), &o2).unwrap_err().contains("status date"));
        let mut r3 = parse_rows("name,duration_days\nA,3\n").unwrap();
        assert!(apply_progress(&mut r3, "id,actual_finish\n1,2026-10-20\n").is_err());
        assert!(apply_progress(&mut r3, "id,percent_complete\n1,100\n").is_err());
        assert!(apply_progress(&mut r3, "name,actual_start\nNope,2026-10-20\n").is_err());
    }

    #[test]
    fn leveling_within_float() {
        // Bob is double-booked on B and C; C has 5 days of float, so it moves after B and the finish holds.
        let csv = "name,start,duration_days,predecessors,resources\nA,2026-11-02,10,,Ann\nB,2026-11-02,3,,Bob\nC,2026-11-02,3,,Bob\nDone,,0,1;2;3,\n";
        let rows = parse_rows(csv).unwrap();
        let cal = CalendarSpec::default();
        let o = ScheduleOptions { level: true, ..Default::default() };
        let (p0, _) = schedule_full(&rows, None, d("2026-01-01"), &cal, &ScheduleOptions::default()).unwrap();
        let (p, info) = schedule_full(&rows, None, d("2026-01-01"), &cal, &o).unwrap();
        assert_eq!(info.leveled, vec![3]);
        assert!(info.unresolved.is_empty());
        assert_eq!(fmt_dt(&p.tasks[2].start), "2026-11-05 08:00");
        assert_eq!(p.finish, p0.finish);
        assert!(p.tasks[2].notes.as_deref().unwrap().contains("leveled"));
        // Without enough float the conflict is reported, not resolved.
        let tight = "name,start,duration_days,predecessors,resources\nB,2026-11-02,3,,Bob\nC,2026-11-02,3,,Bob\nDone,,0,1;2,\n";
        let (_, info) = schedule_full(&parse_rows(tight).unwrap(), None, d("2026-01-01"), &cal, &o).unwrap();
        assert!(info.leveled.is_empty() && info.unresolved.len() == 1);
    }
}
