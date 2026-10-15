//! Weekly resource loading (work hours per resource per week) with over-allocation flags.
use crate::gantt::esc;
use crate::model::Plan;
use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Serialize, Clone, Debug)]
pub struct ResourceLoad {
    pub uid: i32,
    pub name: String,
    /// Available hours per week (max units × the plan's hours per week).
    pub capacity_hours_per_week: f64,
    pub total_hours: f64,
    pub peak_week_hours: f64,
    /// Number of weeks with load above capacity.
    pub over_allocated_weeks: usize,
    /// Hours per week, keyed by the week's Monday (only weeks with work).
    pub weeks: BTreeMap<NaiveDate, f64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct LoadReport {
    /// Every week (Monday) from the first to the last week with work.
    pub weeks: Vec<NaiveDate>,
    pub resources: Vec<ResourceLoad>,
}

fn monday(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

fn is_work(d: NaiveDate) -> bool {
    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

/// Spread each assignment's work evenly over its Mon–Fri working days and sum per resource and week.
/// Material and cost resources are excluded.
pub fn load(plan: &Plan) -> LoadReport {
    let hpw = if plan.minutes_per_week > 0 { plan.minutes_per_week as f64 / 60.0 } else { 40.0 };
    let tasks: HashMap<i32, &crate::model::TaskRec> = plan.tasks.iter().map(|t| (t.uid, t)).collect();
    let mut per: BTreeMap<i32, BTreeMap<NaiveDate, f64>> = BTreeMap::new();
    for a in &plan.assignments {
        if a.work_hours <= 0.0 {
            continue;
        }
        let t = tasks.get(&a.task_uid);
        let (Some(s), Some(f)) = (a.start.or(t.and_then(|t| t.start)), a.finish.or(t.and_then(|t| t.finish))) else { continue };
        let (s, f) = (s.date(), f.date().max(s.date()));
        let days: Vec<NaiveDate> = s.iter_days().take_while(|d| *d <= f).filter(|d| is_work(*d)).collect();
        let days = if days.is_empty() { vec![s] } else { days };
        let per_day = a.work_hours / days.len() as f64;
        let m = per.entry(a.resource_uid).or_default();
        for d in days {
            *m.entry(monday(d)).or_default() += per_day;
        }
    }
    let mut all = BTreeSet::new();
    let mut resources = Vec::new();
    for r in &plan.resources {
        if r.rtype.contains("Material") || r.rtype.contains("Cost") {
            continue;
        }
        let weeks: BTreeMap<NaiveDate, f64> =
            per.remove(&r.uid).unwrap_or_default().into_iter().map(|(k, v)| (k, (v * 100.0).round() / 100.0)).collect();
        let cap = r.max_units.unwrap_or(100.0) / 100.0 * hpw;
        all.extend(weeks.keys().copied());
        resources.push(ResourceLoad {
            uid: r.uid,
            name: r.name.clone(),
            capacity_hours_per_week: (cap * 100.0).round() / 100.0,
            total_hours: (weeks.values().sum::<f64>() * 100.0).round() / 100.0,
            peak_week_hours: weeks.values().copied().fold(0.0, f64::max),
            over_allocated_weeks: weeks.values().filter(|h| **h > cap + 0.01).count(),
            weeks,
        });
    }
    let weeks = match (all.first(), all.last()) {
        (Some(a), Some(b)) => {
            let mut v = vec![];
            let mut d = *a;
            while d <= *b {
                v.push(d);
                d += Duration::days(7);
            }
            v
        }
        _ => vec![],
    };
    LoadReport { weeks, resources }
}

/// Long-format CSV: one row per resource and week.
pub fn to_csv(r: &LoadReport) -> Result<String, String> {
    let mut w = csv::Writer::from_writer(vec![]);
    w.write_record(["resource_uid", "resource", "week_start", "hours", "capacity_hours", "over_allocated"]).map_err(|e| e.to_string())?;
    for res in &r.resources {
        for wk in &r.weeks {
            let h = res.weeks.get(wk).copied().unwrap_or(0.0);
            w.write_record([
                res.uid.to_string(),
                res.name.clone(),
                wk.to_string(),
                h.to_string(),
                res.capacity_hours_per_week.to_string(),
                (h > res.capacity_hours_per_week + 0.01).to_string(),
            ])
            .map_err(|e| e.to_string())?;
        }
    }
    String::from_utf8(w.into_inner().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Self-contained HTML heat map.
pub fn to_html(r: &LoadReport, title: &str) -> String {
    let mut head = String::new();
    for w in &r.weeks {
        head += &format!("<th title=\"week of {w}\">{}</th>", w.format("%d %b<br>%Y"));
    }
    let mut rows = String::new();
    for res in &r.resources {
        let cap = res.capacity_hours_per_week;
        let mut cells = String::new();
        for w in &r.weeks {
            let h = res.weeks.get(w).copied().unwrap_or(0.0);
            let (cls, alpha) = if h > cap + 0.01 { ("over", 1.0) } else { ("", if cap > 0.0 { (h / cap).min(1.0) } else { 0.0 }) };
            let style = if cls.is_empty() && h > 0.0 {
                format!(" style=\"background:rgba(30,136,229,{:.2})\"", 0.12 + alpha * 0.5)
            } else {
                String::new()
            };
            let txt = if h > 0.0 { format!("{h:.0}") } else { String::new() };
            cells += &format!("<td class=\"{cls}\"{style} title=\"{} · week of {w}: {h:.1} h of {cap:.0} h\">{txt}</td>", esc(&res.name));
        }
        rows += &format!(
            "<tr><th class=\"name\">{}</th><td class=\"num\">{cap:.0}</td><td class=\"num\">{:.0}</td><td class=\"num{}\">{}</td>{cells}</tr>\n",
            esc(&res.name),
            res.total_hours,
            if res.over_allocated_weeks > 0 { " overtxt" } else { "" },
            res.over_allocated_weeks
        );
    }
    let over: usize = r.resources.iter().filter(|x| x.over_allocated_weeks > 0).count();
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>{t} — resource load</title><style>body{{font-family:Segoe UI,Arial,sans-serif;margin:16px}}table{{border-collapse:collapse;font-size:12px}}td,th{{border:1px solid #e0e0e0;padding:2px 5px;text-align:right;min-width:26px}}th{{background:#f5f5f5;font-weight:600}}th.name{{text-align:left;white-space:nowrap;position:sticky;left:0}}td.over{{background:#c62828;color:#fff;font-weight:bold}}.overtxt{{color:#c62828;font-weight:bold}}.wrap{{overflow:auto}}</style></head><body>\n<h1>{t} — weekly resource load</h1><p>{} resources, {} weeks. {over} over-allocated (red cells: hours above weekly capacity).</p>\n<div class=\"wrap\"><table><tr><th class=\"name\">Resource</th><th>Cap h/wk</th><th>Total h</th><th>Over wks</th>{head}</tr>\n{rows}</table></div></body></html>\n",
        r.resources.len(),
        r.weeks.len(),
        t = esc(title)
    )
}
