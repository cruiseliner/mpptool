//! Build a new schedule from a simple CSV and compute dates on a Mon–Fri, 08:00–17:00 calendar.
//!
//! Columns (header required, case-insensitive): `name, start, duration_days, predecessors, outline_level [, resources, notes]`
//! - `start`: `YYYY-MM-DD` (optional) — saved as a Start-No-Earlier-Than constraint. The earliest start is the
//!   project start; if no row has a start, the project starts today.
//! - `predecessors`: row numbers (= task IDs, 1-based) separated by `;` or `,` with optional type and lag in
//!   working days, e.g. `2`, `2FS`, `3SS+2`, `4FF-1d`, `2;3`.
//! - `outline_level`: 1 = top level (default). A row followed by deeper rows becomes a summary task.
//! - `resources`: optional, names separated by `;`. `notes`: optional free text.
//!
//! After the forward pass a backward pass computes total slack; tasks with zero slack are flagged critical.
use crate::model::*;
use chrono::{Datelike, Duration as CDur, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
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
}

fn is_work(d: NaiveDate) -> bool {
    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

fn next_work(mut d: NaiveDate) -> NaiveDate {
    while !is_work(d) {
        d = d.succ_opt().expect("date overflow");
    }
    d
}

/// Working-day index of `d` relative to `base` (a working day). Non-working dates roll forward.
fn day_index(base: NaiveDate, d: NaiveDate) -> f64 {
    let d = next_work(d);
    let (mut i, mut c) = (0i64, base);
    while c < d {
        c = c.succ_opt().expect("date overflow");
        if is_work(c) {
            i += 1;
        }
    }
    while c > d {
        c = c.pred_opt().expect("date overflow");
        if is_work(c) {
            i -= 1;
        }
    }
    i as f64
}

fn date_of(base: NaiveDate, mut i: i64) -> NaiveDate {
    let mut c = base;
    while i > 0 {
        c = c.succ_opt().expect("date overflow");
        if is_work(c) {
            i -= 1;
        }
    }
    while i < 0 {
        c = c.pred_opt().expect("date overflow");
        if is_work(c) {
            i += 1;
        }
    }
    c
}

/// Convert a fractional working-day position into a timestamp (08:00–12:00, 13:00–17:00).
/// For finishes, a whole-day boundary maps to 17:00 of the previous working day.
fn to_dt(base: NaiveDate, x: f64, is_finish: bool) -> NaiveDateTime {
    let mut day = x.floor() as i64;
    let mut frac = x - x.floor();
    if is_finish && frac < 1e-9 && x > 1e-9 {
        day -= 1;
        frac = 1.0;
    }
    let mut mins = (frac * 480.0).round() as i64;
    if mins > 240 {
        mins += 60;
    }
    date_of(base, day).and_time(NaiveTime::from_hms_opt(8, 0, 0).expect("valid time")) + CDur::minutes(mins)
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
        rows.push(Row { name, start, dur, preds, level, res, notes });
    }
    if rows.is_empty() {
        return Err("CSV has no tasks".into());
    }
    Ok(rows)
}

/// Read a CSV file and schedule it.
pub fn build(path: &Path) -> Result<Plan, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path.file_stem().map(|s| s.to_string_lossy().to_string());
    schedule(&parse_rows(&text)?, name, chrono::Local::now().date_naive())
}

/// Schedule rows into a plan (forward pass, summary roll-up, backward pass for slack/critical path).
pub fn schedule(rows: &[Row], name: Option<String>, today: NaiveDate) -> Result<Plan, String> {
    let n = rows.len();
    for (i, r) in rows.iter().enumerate() {
        for (p, _, _) in &r.preds {
            if *p == 0 || *p > n || *p == i + 1 {
                return Err(format!("task {} '{}': predecessor {} out of range", i + 1, r.name, p));
            }
        }
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
    let children = |i: usize| (i + 1..n).take_while(move |&k| rows[k].level > rows[i].level);
    let leaves_of = |i: usize| -> Vec<usize> { if summary[i] { children(i).filter(|&k| !summary[k]).collect() } else { vec![i] } };
    let dur = |i: usize| if summary[i] { 0.0 } else { rows[i].dur };

    // Expanded dependency edges between leaf tasks: (pred_leaf, succ_leaf, type, lag_days).
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
                    edges.push((pl, i, ty.clone(), *lag));
                }
            }
        }
    }

    let pstart = next_work(rows.iter().filter_map(|r| r.start).min().unwrap_or(today));
    let snet: Vec<f64> = rows.iter().map(|r| r.start.map(|d| day_index(pstart, d)).unwrap_or(0.0)).collect();
    let (mut es, mut ef) = (snet.clone(), vec![0.0f64; n]);
    for i in 0..n {
        ef[i] = es[i] + dur(i);
    }
    let mut converged = false;
    for _ in 0..(n * 2 + 10) {
        let mut changed = false;
        for &(p, s, ref ty, lag) in &edges {
            let d = dur(s);
            let v = match ty.as_str() {
                "SS" => es[p] + lag,
                "FF" => ef[p] + lag - d,
                "SF" => es[p] + lag - d,
                _ => ef[p] + lag,
            };
            if v > es[s] + EPS {
                es[s] = v;
                ef[s] = v + d;
                changed = true;
            }
        }
        if !changed {
            converged = true;
            break;
        }
    }
    if !converged {
        return Err("could not resolve schedule (circular dependencies?)".into());
    }
    for i in (0..n).rev().filter(|&i| summary[i]) {
        let k: Vec<usize> = children(i).collect();
        es[i] = k.iter().map(|&k| es[k]).fold(f64::MAX, f64::min);
        ef[i] = k.iter().map(|&k| ef[k]).fold(f64::MIN, f64::max);
    }

    // Backward pass over leaves.
    let pfinish = (0..n).filter(|&i| !summary[i]).map(|i| ef[i]).fold(0.0, f64::max);
    let (mut lf, mut ls) = (vec![pfinish; n], vec![0.0; n]);
    for i in 0..n {
        ls[i] = lf[i] - dur(i);
    }
    for _ in 0..(n * 2 + 10) {
        let mut changed = false;
        for &(p, s, ref ty, lag) in &edges {
            let d = dur(p);
            let v = match ty.as_str() {
                "SS" => ls[s] - lag + d,
                "FF" => lf[s] - lag,
                "SF" => lf[s] - lag + d,
                _ => ls[s] - lag,
            };
            if v < lf[p] - EPS {
                lf[p] = v;
                ls[p] = v - d;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut slack = vec![0.0; n];
    let mut critical = vec![false; n];
    for i in (0..n).filter(|&i| !summary[i]) {
        slack[i] = (ls[i] - es[i]).max(0.0);
        critical[i] = slack[i] < EPS;
    }
    for i in (0..n).rev().filter(|&i| summary[i]) {
        let k: Vec<usize> = children(i).collect();
        critical[i] = k.iter().any(|&k| critical[k]);
        slack[i] = k.iter().map(|&k| slack[k]).fold(f64::MAX, f64::min);
    }

    let mut plan = Plan {
        format: "CSV".into(),
        name: name.clone(),
        title: name,
        minutes_per_day: 480,
        minutes_per_week: 2400,
        default_calendar_uid: Some(1),
        calendars: vec![CalendarRec::standard(1)],
        ..Default::default()
    };
    let mut res_uid: HashMap<String, i32> = HashMap::new();
    let midnight8 = NaiveTime::from_hms_opt(8, 0, 0).expect("valid time");
    for (i, r) in rows.iter().enumerate() {
        let ms = !summary[i] && r.dur == 0.0;
        let start = to_dt(pstart, es[i], ms && es[i] > 0.0);
        let finish = if ms { start } else { to_dt(pstart, ef[i], true) };
        plan.tasks.push(TaskRec {
            uid: (i + 1) as i32,
            id: Some((i + 1) as i32),
            name: r.name.clone(),
            outline_level: r.level,
            start: Some(start),
            finish: Some(finish),
            duration_hours: (ef[i] - es[i]) * 8.0,
            milestone: ms,
            summary: summary[i],
            critical: critical[i],
            total_slack_hours: Some(slack[i] * 8.0),
            notes: r.notes.clone(),
            constraint_type: r.start.map(|_| 4),
            constraint_date: r.start.map(|d| next_work(d).and_time(midnight8)),
            predecessors: r
                .preds
                .iter()
                .map(|(p, ty, lag)| Pred {
                    predecessor_uid: *p as i32,
                    predecessor_id: Some(*p as i32),
                    link_type: ty.clone(),
                    lag_hours: lag * 8.0,
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
                    work_hours: r.dur * 8.0,
                    start: Some(start),
                    finish: Some(finish),
                    cost: None,
                });
            }
        }
    }
    plan.start = plan.tasks.iter().filter_map(|t| t.start).min();
    plan.finish = plan.tasks.iter().filter_map(|t| t.finish).max();
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
        let base = d("2026-10-16");
        assert_eq!(date_of(base, 1), d("2026-10-19"));
        assert_eq!(day_index(base, d("2026-10-17")), 1.0); // Saturday rolls to Monday
        assert_eq!(to_dt(base, 1.0, true).to_string(), "2026-10-16 17:00:00");
        assert_eq!(to_dt(base, 0.5, false).to_string(), "2026-10-16 12:00:00");
        assert_eq!(to_dt(base, 0.75, false).to_string(), "2026-10-16 15:00:00");
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
