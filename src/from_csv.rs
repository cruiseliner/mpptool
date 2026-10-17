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

#[derive(Debug, Clone, PartialEq)]
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
        rows.push(Row { name, start, dur, preds, level, res, notes, optimistic, pessimistic });
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
}

impl Network {
    /// Build the network (validates references and detects cycles).
    pub fn new(rows: &[Row], today: NaiveDate, cal: &CalendarSpec) -> Result<Network, String> {
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
        let first = rows.iter().filter_map(|r| r.start).min().unwrap_or(today);
        let wc = cal.resolve(first.year())?;
        let mut timeline = Timeline::new(wc, first);
        let snet = rows.iter().map(|r| r.start.map(|d| timeline.pos_of_date(d)).unwrap_or(0.0)).collect();
        Ok(Network { n, summary, edges, snet, topo, incoming, outgoing, timeline, minutes_per_day: mpd })
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
        let (mut es, mut ef) = (self.snet.clone(), vec![0.0; self.n]);
        for &s in &self.topo {
            let mut v = self.snet[s];
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
    let mut net = Network::new(rows, today, cal)?;
    let n = net.n;
    let summary = net.summary.clone();
    let mpd = net.minutes_per_day;
    let dur: Vec<f64> = (0..n).map(|i| if summary[i] { 0.0 } else { rows[i].dur * mpd }).collect();
    let (mut es, mut ef) = net.forward(&dur);
    let children = |i: usize| (i + 1..n).take_while(move |&k| rows[k].level > rows[i].level);
    for i in (0..n).rev().filter(|&i| summary[i]) {
        let k: Vec<usize> = children(i).collect();
        es[i] = k.iter().map(|&k| es[k]).fold(f64::MAX, f64::min);
        ef[i] = k.iter().map(|&k| ef[k]).fold(f64::MIN, f64::max);
    }

    // Backward pass over leaves (reverse topological order).
    let pfinish = (0..n).filter(|&i| !summary[i]).map(|i| ef[i]).fold(0.0, f64::max);
    let (mut lf, mut ls) = (vec![pfinish; n], vec![0.0; n]);
    let mut free = vec![f64::MAX; n];
    for i in 0..n {
        ls[i] = lf[i] - dur[i];
    }
    for &p in net.topo.iter().rev() {
        ls[p] = lf[p] - dur[p];
        // Successors of p are already final; tighten p by every outgoing edge.
        for &k in &net.outgoing[p] {
            let (_, s, ref ty, lag) = net.edges[k];
            let d = dur[p];
            let v = match ty.as_str() {
                "SS" => ls[s] - lag + d,
                "FF" => lf[s] - lag,
                "SF" => lf[s] - lag + d,
                _ => ls[s] - lag,
            };
            if v < lf[p] {
                lf[p] = v;
                ls[p] = v - d;
            }
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
    let mut slack = vec![0.0; n];
    let mut critical = vec![false; n];
    for i in (0..n).filter(|&i| !summary[i]) {
        slack[i] = (ls[i] - es[i]).max(0.0);
        free[i] = free[i].max(0.0).min(slack[i]);
        critical[i] = slack[i] < EPS;
    }
    for i in (0..n).rev().filter(|&i| summary[i]) {
        let k: Vec<usize> = children(i).collect();
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
            notes: r.notes.clone(),
            constraint_type: r.start.map(|_| 4),
            constraint_date: r.start.map(|d| tl.first_work_at(d)),
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
    Ok(plan)
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
}
