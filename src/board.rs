//! Kanban board (Not started / In progress / Done) with RAG flags.
use crate::gantt::esc;
use crate::model::{Plan, TaskRec};
use crate::workdays::slip_days_cal;
use chrono::NaiveDateTime;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Rag {
    Red,
    Amber,
    Green,
}

#[derive(Serialize, Clone, Debug)]
pub struct Card {
    pub uid: i32,
    pub key: String,
    pub name: String,
    /// Name of the parent summary task.
    pub parent: Option<String>,
    pub status: String,
    pub rag: Rag,
    /// Why the card is red or amber.
    pub reasons: Vec<String>,
    pub start: Option<NaiveDateTime>,
    pub finish: Option<NaiveDateTime>,
    pub percent_complete: f64,
    pub critical: bool,
    pub milestone: bool,
    pub resources: Vec<String>,
}

pub const COLUMNS: [&str; 3] = ["Not started", "In progress", "Done"];

/// RAG rules (evaluated at `at`, the status date or today):
/// red = unfinished and past its finish, finish > 5 working days later than baseline, or negative float;
/// amber = not started although its start has passed, finish 1–5 working days later than baseline, or a
/// critical task in progress; green otherwise. Done tasks are green.
pub fn cards(plan: &Plan, at: NaiveDateTime) -> Vec<Card> {
    let cal = crate::calendar::plan_calendar(plan);
    let names: HashMap<i32, &str> = plan.resources.iter().map(|r| (r.uid, r.name.as_str())).collect();
    let mut res: HashMap<i32, Vec<String>> = HashMap::new();
    for a in &plan.assignments {
        if let Some(n) = names.get(&a.resource_uid) {
            res.entry(a.task_uid).or_default().push((*n).to_string());
        }
    }
    let mut parents: Vec<&TaskRec> = Vec::new();
    let mut out = Vec::new();
    for t in &plan.tasks {
        while parents.last().is_some_and(|p| p.outline_level >= t.outline_level) {
            parents.pop();
        }
        if t.summary {
            parents.push(t);
            continue;
        }
        if crate::mspdi_writer::is_null_task(t) {
            continue;
        }
        let done = t.percent_complete >= 100.0 || t.actual_finish.is_some();
        let started = t.percent_complete > 0.0 || t.actual_start.is_some();
        let status = if done {
            COLUMNS[2]
        } else if started {
            COLUMNS[1]
        } else {
            COLUMNS[0]
        };
        let mut red = vec![];
        let mut amber = vec![];
        if !done {
            if t.finish.is_some_and(|f| f < at) {
                red.push("overdue".to_string());
            }
            if t.total_slack_hours.is_some_and(|f| f < -0.01) {
                red.push("negative float".into());
            }
            match slip_days_cal(&cal, t.baseline_finish, t.finish) {
                Some(d) if d > 5 => red.push(format!("{d} wd late vs baseline")),
                Some(d) if d > 0 => amber.push(format!("{d} wd late vs baseline")),
                _ => {}
            }
            if !started && t.start.is_some_and(|s| s < at) {
                amber.push("start date passed".into());
            }
            if started && t.critical {
                amber.push("critical".into());
            }
        }
        let rag = if !red.is_empty() {
            Rag::Red
        } else if !amber.is_empty() {
            Rag::Amber
        } else {
            Rag::Green
        };
        red.extend(amber);
        out.push(Card {
            uid: t.uid,
            key: t.activity_id.clone().unwrap_or_else(|| t.id.unwrap_or(t.uid).to_string()),
            name: t.name.clone(),
            parent: parents.last().map(|p| p.name.clone()),
            status: status.into(),
            rag,
            reasons: red,
            start: t.start,
            finish: t.finish,
            percent_complete: t.percent_complete,
            critical: t.critical,
            milestone: t.milestone,
            resources: res.remove(&t.uid).unwrap_or_default(),
        });
    }
    out
}

/// Self-contained HTML board.
pub fn to_html(plan: &Plan, cards: &[Card], at: NaiveDateTime, title: Option<&str>) -> String {
    let title = title.map(String::from).or(plan.title.clone()).or(plan.name.clone()).unwrap_or_else(|| "Project".into());
    let d = |x: Option<NaiveDateTime>| x.map(|x| x.format("%d %b %Y").to_string()).unwrap_or_default();
    let mut cols = String::new();
    for col in COLUMNS {
        let mine: Vec<&Card> = cards.iter().filter(|c| c.status == col).collect();
        let count = |r: Rag| mine.iter().filter(|c| c.rag == r).count();
        let mut body = String::new();
        for c in &mine {
            let rag = match c.rag {
                Rag::Red => "red",
                Rag::Amber => "amber",
                Rag::Green => "green",
            };
            body += &format!(
                "<div class=\"card {rag}\" title=\"{}\"><div class=\"top\"><span class=\"key\">{}</span>{}{}<span class=\"rag {rag}\">{}</span></div><div class=\"name\">{}</div>{}<div class=\"dates\">{} → {}</div><div class=\"bar\"><i style=\"width:{:.0}%\"></i></div><div class=\"meta\">{:.0}%{}</div>{}</div>\n",
                esc(&c.reasons.join("; ")),
                esc(&c.key),
                if c.critical { "<span class=\"tag crit\">critical</span>" } else { "" },
                if c.milestone { "<span class=\"tag\">◆ milestone</span>" } else { "" },
                rag.to_uppercase(),
                esc(&c.name),
                c.parent.as_ref().map(|p| format!("<div class=\"parent\">{}</div>", esc(p))).unwrap_or_default(),
                d(c.start),
                d(c.finish),
                c.percent_complete.clamp(0.0, 100.0),
                c.percent_complete,
                if c.resources.is_empty() { String::new() } else { format!(" · {}", esc(&c.resources.join(", "))) },
                if c.reasons.is_empty() { String::new() } else { format!("<div class=\"why\">{}</div>", esc(&c.reasons.join(" · "))) }
            );
        }
        cols += &format!(
            "<section class=\"col\"><h2>{col} <span class=\"n\">{}</span></h2><div class=\"sum\"><span class=\"dot red\"></span>{} <span class=\"dot amber\"></span>{} <span class=\"dot green\"></span>{}</div>\n{body}</section>\n",
            mine.len(),
            count(Rag::Red),
            count(Rag::Amber),
            count(Rag::Green)
        );
    }
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>{t} — board</title><style>\
body{{margin:0;font-family:Segoe UI,Arial,sans-serif;background:#f4f5f7}}header{{padding:10px 16px;background:#fff;border-bottom:1px solid #ddd}}h1{{font-size:18px;margin:0}}.sub{{color:#666;font-size:12px}}\
.board{{display:flex;gap:12px;padding:12px;align-items:flex-start}}.col{{flex:1;min-width:260px;background:#ebecf0;border-radius:6px;padding:8px}}h2{{font-size:14px;margin:4px 4px 2px}}.n{{color:#666;font-weight:normal}}.sum{{font-size:11px;color:#555;margin:0 4px 8px}}\
.card{{background:#fff;border-radius:5px;padding:7px 9px;margin-bottom:8px;box-shadow:0 1px 1px rgba(0,0,0,.15);border-left:5px solid #43a047;font-size:12px}}.card.red{{border-left-color:#d32f2f}}.card.amber{{border-left-color:#f9a825}}\
.top{{display:flex;gap:6px;align-items:center}}.key{{color:#777;font-size:11px}}.rag{{margin-left:auto;font-size:10px;font-weight:bold;padding:1px 5px;border-radius:3px;color:#fff}}.rag.red{{background:#d32f2f}}.rag.amber{{background:#f9a825}}.rag.green{{background:#43a047}}\
.tag{{font-size:10px;background:#eee;border-radius:3px;padding:0 4px}}.tag.crit{{background:#ffebee;color:#c62828}}.name{{font-weight:600;margin:3px 0}}.parent{{color:#777;font-size:11px}}.dates,.meta{{color:#555;font-size:11px}}\
.bar{{height:4px;background:#eee;border-radius:2px;margin:4px 0}}.bar i{{display:block;height:4px;background:#1e88e5;border-radius:2px}}.why{{color:#b71c1c;font-size:11px;margin-top:3px}}.dot{{display:inline-block;width:8px;height:8px;border-radius:4px;margin:0 2px 0 6px}}.dot.red{{background:#d32f2f}}.dot.amber{{background:#f9a825}}.dot.green{{background:#43a047}}\
</style></head><body><header><h1>{t}</h1><div class=\"sub\">{} tasks · RAG as of {} · red: overdue, &gt; 5 wd late vs baseline or negative float · amber: late start, 1–5 wd late, or critical in progress</div></header>\n<main class=\"board\">\n{cols}</main></body></html>\n",
        cards.len(),
        at.format("%Y-%m-%d"),
        t = esc(&title)
    )
}
