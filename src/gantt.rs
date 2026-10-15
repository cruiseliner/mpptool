//! Self-contained Gantt chart rendering (SVG, optionally wrapped in an offline HTML page).
use crate::model::{Plan, TaskRec};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};
use std::collections::HashMap;
use std::fmt::Write;

/// Options for [`render_svg`] / [`render_html`].
#[derive(Debug, Clone, Default)]
pub struct GanttOptions {
    /// Only show critical tasks (plus their summary ancestors).
    pub critical_only: bool,
    /// Visible window start (tasks entirely outside the window are hidden).
    pub from: Option<NaiveDate>,
    /// Visible window end.
    pub to: Option<NaiveDate>,
    /// Chart title; defaults to the project title or name.
    pub title: Option<String>,
    /// Date of the "today" line; defaults to the current local date.
    pub today: Option<NaiveDate>,
}

const ROW: f64 = 22.0;
const LABEL_W: f64 = 380.0;
const HEAD: f64 = 64.0;
const MAX_CHART_W: f64 = 24000.0;

/// Escape text for XML/HTML.
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Scale {
    Days,
    Weeks,
    Months,
    Quarters,
}

fn pick_scale(days: i64) -> (Scale, f64) {
    match days {
        ..=45 => (Scale::Days, 28.0),
        46..=200 => (Scale::Weeks, 8.0),
        201..=900 => (Scale::Months, 2.5),
        _ => (Scale::Quarters, (6000.0 / days as f64).clamp(0.4, 1.2)),
    }
}

fn task_window(t: &TaskRec) -> Option<(NaiveDateTime, NaiveDateTime)> {
    let s = t.start.or(t.finish)?;
    let f = t.finish.unwrap_or(s).max(s);
    Some((s, f))
}

/// Select the rows to draw, in plan order.
fn visible_rows<'a>(plan: &'a Plan, o: &GanttOptions) -> Vec<&'a TaskRec> {
    let from = o.from.map(|d| d.and_hms_opt(0, 0, 0).unwrap());
    let to = o.to.map(|d| d.and_hms_opt(23, 59, 59).unwrap());
    let in_window = |t: &TaskRec| match task_window(t) {
        Some((s, f)) => from.is_none_or(|w| f >= w) && to.is_none_or(|w| s <= w),
        None => from.is_none() && to.is_none(),
    };
    let keep_leaf = |t: &TaskRec| in_window(t) && (!o.critical_only || t.critical);
    // Keep summaries only when they have a kept descendant (or, unfiltered, when they're in the window).
    let mut keep = vec![false; plan.tasks.len()];
    let mut stack: Vec<usize> = Vec::new(); // indices of open summary ancestors
    for (i, t) in plan.tasks.iter().enumerate() {
        while let Some(&j) = stack.last() {
            if plan.tasks[j].outline_level >= t.outline_level {
                stack.pop();
            } else {
                break;
            }
        }
        if t.summary {
            keep[i] = !o.critical_only && in_window(t) && o.from.is_none() && o.to.is_none();
            stack.push(i);
        } else if keep_leaf(t) {
            keep[i] = true;
            for &j in &stack {
                keep[j] = true;
            }
        }
    }
    plan.tasks.iter().zip(keep).filter(|(_, k)| *k).map(|(t, _)| t).collect()
}

/// Render the chart as a standalone SVG document.
pub fn render_svg(plan: &Plan, o: &GanttOptions) -> String {
    let rows = visible_rows(plan, o);
    let today = o.today.unwrap_or_else(|| chrono::Local::now().date_naive());
    // Date range.
    let mut lo: Option<NaiveDateTime> = None;
    let mut hi: Option<NaiveDateTime> = None;
    for t in &rows {
        for d in [t.start, t.finish, t.baseline_start, t.baseline_finish].into_iter().flatten() {
            lo = Some(lo.map_or(d, |x| x.min(d)));
            hi = Some(hi.map_or(d, |x| x.max(d)));
        }
    }
    let mut d0 = o.from.or(lo.map(|d| d.date())).unwrap_or(today);
    let mut d1 = o.to.or(hi.map(|d| d.date())).unwrap_or(d0);
    if d1 < d0 {
        std::mem::swap(&mut d0, &mut d1);
    }
    let span = (d1 - d0).num_days().max(1);
    let (scale, mut ppd) = pick_scale(span);
    // Pad to scale boundaries.
    let (d0, d1) = match scale {
        Scale::Days => (d0 - Duration::days(1), d1 + Duration::days(2)),
        Scale::Weeks => (d0 - Duration::days(d0.weekday().num_days_from_monday() as i64), d1 + Duration::days(8)),
        Scale::Months | Scale::Quarters => (
            NaiveDate::from_ymd_opt(d0.year(), d0.month(), 1).unwrap(),
            next_month(NaiveDate::from_ymd_opt(d1.year(), d1.month(), 1).unwrap()),
        ),
    };
    let days = (d1 - d0).num_days().max(1) as f64;
    if days * ppd > MAX_CHART_W {
        ppd = MAX_CHART_W / days;
    }
    let chart_w = (days * ppd).ceil();
    let width = LABEL_W + chart_w + 20.0;
    let height = HEAD + ROW * rows.len().max(1) as f64 + 30.0;
    let x_of = |d: NaiveDateTime| -> f64 {
        let secs = (d - d0.and_hms_opt(0, 0, 0).unwrap()).num_seconds() as f64;
        LABEL_W + (secs / 86400.0 * ppd).clamp(-5.0, chart_w + 5.0)
    };
    let title = o.title.clone().or(plan.title.clone()).or(plan.name.clone()).unwrap_or_else(|| "Project".into());

    let mut s = String::with_capacity(rows.len() * 400 + 4096);
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0}" height="{height:.0}" viewBox="0 0 {width:.0} {height:.0}" font-family="Segoe UI, Arial, sans-serif" font-size="11">
<defs><marker id="arr" viewBox="0 0 6 6" refX="5" refY="3" markerWidth="6" markerHeight="6" orient="auto"><path d="M0,0L6,3L0,6z" fill="#555"/></marker>
<marker id="arrc" viewBox="0 0 6 6" refX="5" refY="3" markerWidth="6" markerHeight="6" orient="auto"><path d="M0,0L6,3L0,6z" fill="#c62828"/></marker></defs>
<rect width="100%" height="100%" fill="#fff"/>
<text x="8" y="18" font-size="15" font-weight="bold">{}</text>
"##,
        esc(&title)
    );
    // Time scale: upper band (months/years) and lower band (days/weeks/months/quarters) with grid lines.
    let gy = HEAD - 36.0;
    let _ = writeln!(s, r##"<g class="scale" stroke="#ddd"><rect x="{LABEL_W}" y="{gy}" width="{chart_w}" height="36" fill="#f5f5f5"/>"##);
    let mut grid = String::new();
    let mut d = d0;
    while d < d1 {
        let (next, label, major) = match scale {
            Scale::Days => (d + Duration::days(1), d.day().to_string(), d.weekday().num_days_from_monday() == 0),
            Scale::Weeks => (d + Duration::days(7), d.format("%d %b").to_string(), d.day() <= 7),
            Scale::Months => (next_month(d), d.format("%b").to_string(), d.month() == 1),
            Scale::Quarters => (add_months(d, 3), format!("Q{}", (d.month() - 1) / 3 + 1), d.month() == 1),
        };
        let x = x_of(d.and_hms_opt(0, 0, 0).unwrap());
        let w = x_of(next.min(d1).and_hms_opt(0, 0, 0).unwrap()) - x;
        if matches!(scale, Scale::Days) && d.weekday().num_days_from_monday() >= 5 {
            let _ = writeln!(
                grid,
                r##"<rect x="{x:.1}" y="{HEAD}" width="{w:.1}" height="{:.0}" fill="#fafafa" stroke="none"/>"##,
                height - HEAD - 30.0
            );
        }
        let _ = writeln!(
            grid,
            r##"<line x1="{x:.1}" y1="{HEAD}" x2="{x:.1}" y2="{:.0}" stroke="{}"/>"##,
            height - 30.0,
            if major { "#ccc" } else { "#eee" }
        );
        if w >= 14.0 {
            let _ = writeln!(s, r##"<text x="{:.1}" y="{}" fill="#555" stroke="none">{}</text>"##, x + 2.0, HEAD - 4.0, esc(&label));
        }
        d = next;
    }
    // Upper band.
    let mut d = d0;
    while d < d1 {
        let (next, label) = match scale {
            Scale::Days | Scale::Weeks => {
                (next_month(NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap()), d.format("%B %Y").to_string())
            }
            Scale::Months | Scale::Quarters => (NaiveDate::from_ymd_opt(d.year() + 1, 1, 1).unwrap(), d.year().to_string()),
        };
        let x = x_of(d.and_hms_opt(0, 0, 0).unwrap());
        let _ = writeln!(
            s,
            r##"<line x1="{x:.1}" y1="{gy}" x2="{x:.1}" y2="{}"/><text x="{:.1}" y="{}" fill="#333" stroke="none" font-weight="bold">{}</text>"##,
            gy + 18.0,
            x + 3.0,
            gy + 13.0,
            esc(&label)
        );
        d = next;
    }
    s.push_str("</g>\n");
    s.push_str(&grid);
    // Label panel header.
    let _ = writeln!(
        s,
        r##"<rect x="0" y="{gy}" width="{LABEL_W}" height="36" fill="#f5f5f5"/><text x="8" y="{}" font-weight="bold">ID</text><text x="52" y="{}" font-weight="bold">Task</text>"##,
        HEAD - 6.0,
        HEAD - 6.0
    );

    let row_of: HashMap<i32, usize> = rows.iter().enumerate().map(|(i, t)| (t.uid, i)).collect();
    let min_level = rows.iter().map(|t| t.outline_level).min().unwrap_or(1);
    let mut bars = String::new();
    let mut links = String::new();
    let mut labels = String::new();
    let y_of = |i: usize| HEAD + ROW * i as f64;
    for (i, t) in rows.iter().enumerate() {
        let y = y_of(i);
        if i % 2 == 1 {
            let _ = writeln!(labels, r##"<rect x="0" y="{y}" width="{width:.0}" height="{ROW}" fill="#000" fill-opacity="0.025"/>"##);
        }
        let indent = 52.0 + 12.0 * (t.outline_level - min_level).clamp(0, 20) as f64;
        let id = t.activity_id.clone().unwrap_or_else(|| t.id.unwrap_or(t.uid).to_string());
        let max_chars = ((LABEL_W - indent - 6.0) / 6.0).max(4.0) as usize;
        let name = truncate(&t.name, max_chars);
        let _ = writeln!(
            labels,
            r##"<text x="8" y="{:.1}" fill="#777">{}</text><text x="{indent}" y="{:.1}"{}{}>{}</text>"##,
            y + 15.0,
            esc(&truncate(&id, 7)),
            y + 15.0,
            if t.summary { r#" font-weight="bold""# } else { "" },
            if t.critical && !t.summary { r##" fill="#c62828""## } else { "" },
            esc(&name)
        );
        let tip = format!(
            "{} — {}\n{} → {}\n{:.0}% complete{}",
            id,
            t.name,
            crate::model::fmt_dt(&t.start),
            crate::model::fmt_dt(&t.finish),
            t.percent_complete,
            if t.critical { ", critical" } else { "" }
        );
        // Baseline ghost bar.
        if let (Some(bs), Some(bf)) = (t.baseline_start, t.baseline_finish) {
            let (x1, x2) = (x_of(bs), x_of(bf));
            let _ = writeln!(
                bars,
                r##"<rect class="baseline" x="{x1:.1}" y="{:.1}" width="{:.1}" height="4" fill="#9e9e9e" fill-opacity="0.6"/>"##,
                y + 15.0,
                (x2 - x1).max(1.5)
            );
        }
        let Some((st, fi)) = task_window(t) else { continue };
        let (x1, x2) = (x_of(st), x_of(fi));
        let color = if t.critical { "#c62828" } else { "#1e88e5" };
        if t.milestone {
            let (cx, cy) = (x1, y + ROW / 2.0);
            let _ = writeln!(
                bars,
                r##"<path class="milestone" d="M{cx:.1},{:.1}L{:.1},{cy:.1}L{cx:.1},{:.1}L{:.1},{cy:.1}z" fill="{}"><title>{}</title></path>"##,
                cy - 6.0,
                cx + 6.0,
                cy + 6.0,
                cx - 6.0,
                if t.critical { "#c62828" } else { "#333" },
                esc(&tip)
            );
            let _ = writeln!(bars, r##"<text x="{:.1}" y="{:.1}" fill="#555">{}</text>"##, cx + 9.0, cy + 4.0, fi.format("%d %b"));
        } else if t.summary {
            let w = (x2 - x1).max(2.0);
            let _ = writeln!(
                bars,
                r##"<path class="summary" d="M{x1:.1},{:.1}h{w:.1}v9l-4,-4H{:.1}l-4,4z" fill="#37474f"><title>{}</title></path>"##,
                y + 6.0,
                x1 + 4.0,
                esc(&tip)
            );
        } else {
            let w = (x2 - x1).max(2.0);
            let pw = w * (t.percent_complete.clamp(0.0, 100.0) / 100.0);
            let _ = writeln!(
                bars,
                r##"<g class="task{}"><title>{}</title><rect x="{x1:.1}" y="{:.1}" width="{w:.1}" height="10" rx="2" fill="{color}" fill-opacity="0.45"/>{}</g>"##,
                if t.critical { " critical" } else { "" },
                esc(&tip),
                y + 4.0,
                if pw > 0.0 {
                    format!(
                        r##"<rect class="progress" x="{x1:.1}" y="{:.1}" width="{pw:.1}" height="10" rx="2" fill="{color}"/>"##,
                        y + 4.0
                    )
                } else {
                    String::new()
                }
            );
        }
        // Links into this task.
        for p in &t.predecessors {
            let Some(&pi) = row_of.get(&p.predecessor_uid) else { continue };
            let pt = rows[pi];
            let Some((ps, pf)) = task_window(pt) else { continue };
            let from_x = if p.link_type.starts_with('S') { x_of(ps) } else { x_of(pf) };
            let to_x = if p.link_type.ends_with('S') { x1 } else { x2 };
            let (fy, ty) = (y_of(pi) + ROW / 2.0, y + ROW / 2.0);
            let crit = t.critical && pt.critical;
            let mid = from_x + 6.0;
            let _ = writeln!(
                links,
                r##"<path d="M{from_x:.1},{fy:.1}H{mid:.1}V{ty:.1}H{:.1}" fill="none" stroke="{}" stroke-width="0.8" marker-end="url(#{})"/>"##,
                to_x - 1.0,
                if crit { "#c62828" } else { "#888" },
                if crit { "arrc" } else { "arr" }
            );
        }
    }
    s.push_str(&labels);
    let _ = writeln!(s, r##"<line x1="{LABEL_W}" y1="{gy}" x2="{LABEL_W}" y2="{:.0}" stroke="#bbb"/>"##, height - 30.0);
    s.push_str("<g class=\"links\">\n");
    s.push_str(&links);
    s.push_str("</g>\n<g class=\"bars\">\n");
    s.push_str(&bars);
    s.push_str("</g>\n");
    // Today line.
    if today >= d0 && today <= d1 {
        let x = x_of(today.and_hms_opt(12, 0, 0).unwrap());
        let _ = writeln!(
            s,
            r##"<g class="today"><line x1="{x:.1}" y1="{gy}" x2="{x:.1}" y2="{:.0}" stroke="#ff9800" stroke-width="1.5" stroke-dasharray="4,3"/><text x="{:.1}" y="{:.0}" fill="#ff9800">Today</text></g>"##,
            height - 30.0,
            x + 3.0,
            height - 18.0
        );
    }
    // Legend.
    let ly = height - 10.0;
    let _ = writeln!(
        s,
        r##"<g class="legend" font-size="10"><rect x="8" y="{:.0}" width="16" height="8" fill="#1e88e5"/><text x="28" y="{ly:.0}">Task</text><rect x="70" y="{:.0}" width="16" height="8" fill="#c62828"/><text x="90" y="{ly:.0}">Critical</text><rect x="140" y="{:.0}" width="16" height="4" fill="#9e9e9e"/><text x="160" y="{ly:.0}">Baseline</text><path d="M222,{:.0}l5,5l-5,5l-5,-5z" fill="#333"/><text x="232" y="{ly:.0}">Milestone</text><text x="300" y="{ly:.0}" fill="#777">{} rows</text></g>"##,
        ly - 8.0,
        ly - 8.0,
        ly - 6.0,
        ly - 13.0,
        rows.len()
    );
    s.push_str("</svg>\n");
    s
}

/// Render an offline HTML page with the chart inlined (no external assets).
pub fn render_html(plan: &Plan, o: &GanttOptions) -> String {
    let title = o.title.clone().or(plan.title.clone()).or(plan.name.clone()).unwrap_or_else(|| "Project".into());
    let sub = format!(
        "{} · {} → {}{}",
        plan.format,
        crate::model::fmt_dt(&plan.start),
        crate::model::fmt_dt(&plan.finish),
        plan.status_date.map(|d| format!(" · status date {}", d.format("%Y-%m-%d"))).unwrap_or_default()
    );
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>{t}</title>\n<style>body{{margin:0;font-family:Segoe UI,Arial,sans-serif;background:#fff}}header{{padding:8px 12px;border-bottom:1px solid #ddd}}h1{{font-size:18px;margin:0}}.sub{{color:#666;font-size:12px}}.wrap{{overflow:auto;height:calc(100vh - 60px)}}g.task:hover rect,path.milestone:hover,path.summary:hover{{stroke:#000;stroke-width:1}}</style></head>\n<body><header><h1>{t}</h1><div class=\"sub\">{sub}</div></header><div class=\"wrap\">\n{svg}</div></body></html>\n",
        t = esc(&title),
        sub = esc(&sub),
        svg = render_svg(plan, o)
    )
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n.saturating_sub(1)).collect::<String>() + "…" }
}

fn add_months(d: NaiveDate, m: u32) -> NaiveDate {
    let total = d.year() * 12 + d.month0() as i32 + m as i32;
    NaiveDate::from_ymd_opt(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1).unwrap()
}

fn next_month(d: NaiveDate) -> NaiveDate {
    add_months(d, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_adapts() {
        assert_eq!(pick_scale(10).0, Scale::Days);
        assert_eq!(pick_scale(100).0, Scale::Weeks);
        assert_eq!(pick_scale(500).0, Scale::Months);
        assert_eq!(pick_scale(3000).0, Scale::Quarters);
    }

    #[test]
    fn months_wrap() {
        assert_eq!(next_month(NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()), NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
        assert_eq!(add_months(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(), 3), NaiveDate::from_ymd_opt(2027, 2, 1).unwrap());
    }

    #[test]
    fn escapes() {
        assert_eq!(esc("a<b & \"c\""), "a&lt;b &amp; &quot;c&quot;");
    }
}
