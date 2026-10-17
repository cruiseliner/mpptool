//! Compare two versions of a schedule.
use crate::gantt::esc;
use crate::model::{Plan, TaskRec, fmt_dt};
use crate::workdays::slip_days_cal;
use chrono::NaiveDateTime;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};

/// How tasks of the two plans are matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchBy {
    /// Activity ID when both plans have them, otherwise unique ID.
    #[default]
    Auto,
    Uid,
    ActivityId,
    Name,
}

#[derive(Serialize, Clone, Debug)]
pub struct TaskRef {
    pub key: String,
    pub id: Option<i32>,
    pub name: String,
    pub start: Option<NaiveDateTime>,
    pub finish: Option<NaiveDateTime>,
}

#[derive(Serialize, Clone, Debug)]
pub struct FieldChange {
    pub field: String,
    pub old: String,
    pub new: String,
    /// Working-day shift for date fields, hour difference for duration, points for % complete.
    pub delta: Option<f64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct TaskChange {
    pub key: String,
    pub id: Option<i32>,
    pub name: String,
    pub changes: Vec<FieldChange>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Comparison {
    pub matched_by: String,
    pub old_finish: Option<NaiveDateTime>,
    pub new_finish: Option<NaiveDateTime>,
    /// Project finish movement in working days (positive = later).
    pub finish_slip_days: Option<i64>,
    pub old_tasks: usize,
    pub new_tasks: usize,
    pub added: Vec<TaskRef>,
    pub removed: Vec<TaskRef>,
    pub changed: Vec<TaskChange>,
    pub unchanged: usize,
    /// How the matched tasks were keyed, and which tasks fell back from activity-ID matching.
    pub coverage: MatchCoverage,
}

/// Match coverage: a deleted Activity ID alias degrades a task to UID matching silently, so it is counted here.
#[derive(Serialize, Clone, Debug, Default)]
pub struct MatchCoverage {
    /// Matched task pairs keyed by activity ID.
    pub by_activity_id: usize,
    /// Matched task pairs keyed by unique ID (all of them in UID mode; fallbacks in activity-ID mode).
    pub by_uid: usize,
    /// Matched task pairs keyed by name.
    pub by_name: usize,
    /// Tasks without an activity ID in the old / new plan while matching by activity ID (keyed `uid:N` instead).
    pub old_uid_fallback: usize,
    pub new_uid_fallback: usize,
    /// Those tasks, as `old`/`new` refs.
    pub fallback_tasks: Vec<FallbackTask>,
    /// UID-matched pairs whose names differ: likely mis-paired after an MS Project re-save renumbered UIDs.
    pub name_mismatches: Vec<NameMismatch>,
}

#[derive(Serialize, Clone, Debug)]
pub struct NameMismatch {
    pub key: String,
    pub old_name: String,
    pub new_name: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct FallbackTask {
    pub plan: String,
    pub key: String,
    pub id: Option<i32>,
    pub name: String,
}

impl MatchCoverage {
    pub fn fallback(&self) -> usize {
        self.old_uid_fallback + self.new_uid_fallback
    }
    /// One-line summary, e.g. `matched 12 by activity_id, 1 by uid (fallback)`.
    pub fn summary(&self, matched_by: &str) -> String {
        match matched_by {
            "activity_id" => format!("matched {} by activity_id, {} by uid (fallback)", self.by_activity_id, self.by_uid),
            "name" => format!("matched {} by name", self.by_name),
            _ => format!("matched {} by uid", self.by_uid),
        }
    }
    /// Number of findings that make `compare --strict` fail (UID fallbacks + UID-matched name mismatches).
    pub fn strict_failures(&self) -> usize {
        self.fallback() + self.name_mismatches.len()
    }
    /// Warning for UID-matched pairs whose names differ.
    pub fn name_warning(&self) -> Option<String> {
        (!self.name_mismatches.is_empty()).then(|| {
            let mut v: Vec<String> =
                self.name_mismatches.iter().take(10).map(|m| format!("[{}] '{}' → '{}'", m.key, m.old_name, m.new_name)).collect();
            if self.name_mismatches.len() > 10 {
                v.push(format!("… {} more", self.name_mismatches.len() - 10));
            }
            format!(
                "WARNING: {} UID-matched task(s) changed name; check for mis-pairing after UID renumbering: {}",
                self.name_mismatches.len(),
                v.join("; ")
            )
        })
    }
    /// Warning when tasks fell back from activity-ID matching (e.g. an alias was deleted in one file).
    pub fn warning(&self) -> Option<String> {
        (self.fallback() > 0).then(|| {
            let mut names: Vec<String> =
                self.fallback_tasks.iter().take(10).map(|t| format!("{} [{}] {}", t.plan, t.key, t.name)).collect();
            if self.fallback_tasks.len() > 10 {
                names.push(format!("… {} more", self.fallback_tasks.len() - 10));
            }
            format!(
                "WARNING: {} task(s) in the old plan and {} in the new plan have no activity ID and were matched by UID; a deleted alias shows up as removed+added: {}",
                self.old_uid_fallback,
                self.new_uid_fallback,
                names.join("; ")
            )
        })
    }
}

fn key_of(t: &TaskRec, by: MatchBy) -> String {
    match by {
        MatchBy::ActivityId => t.activity_id.clone().filter(|a| !a.trim().is_empty()).unwrap_or_else(|| format!("uid:{}", t.uid)),
        MatchBy::Name => t.name.trim().to_lowercase(),
        _ => t.uid.to_string(),
    }
}

fn keyed(p: &Plan, by: MatchBy) -> (Vec<String>, HashMap<String, &TaskRec>) {
    let mut order = Vec::new();
    let mut map = HashMap::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for t in p.tasks.iter().filter(|t| !(t.uid == 0 && t.outline_level == 0)) {
        let mut k = key_of(t, by);
        let n = seen.entry(k.clone()).or_default();
        *n += 1;
        if *n > 1 {
            k = format!("{k}#{n}");
        }
        order.push(k.clone());
        map.insert(k, t);
    }
    (order, map)
}

fn tref(k: &str, t: &TaskRec) -> TaskRef {
    TaskRef { key: k.into(), id: t.id, name: t.name.clone(), start: t.start, finish: t.finish }
}

fn links(t: &TaskRec, keys: &HashMap<i32, String>) -> BTreeSet<String> {
    t.predecessors
        .iter()
        .map(|p| {
            let k = keys.get(&p.predecessor_uid).cloned().unwrap_or_else(|| format!("uid:{}", p.predecessor_uid));
            let lag = if p.lag_hours != 0.0 { format!("{:+}h", (p.lag_hours * 100.0).round() / 100.0) } else { String::new() };
            format!("{k} {}{lag}", p.link_type)
        })
        .collect()
}

/// Compare `old` with `new`.
pub fn compare(old: &Plan, new: &Plan, by: MatchBy) -> Comparison {
    // Slip is counted on the new plan's project calendar (holidays and custom work weeks honoured).
    let cal = crate::calendar::plan_calendar(new);
    let by = match by {
        MatchBy::Auto if old.tasks.iter().any(|t| t.activity_id.is_some()) && new.tasks.iter().any(|t| t.activity_id.is_some()) => {
            MatchBy::ActivityId
        }
        MatchBy::Auto => MatchBy::Uid,
        b => b,
    };
    let (old_order, old_map) = keyed(old, by);
    let (new_order, new_map) = keyed(new, by);
    let uid_keys = |order: &[String], map: &HashMap<String, &TaskRec>| -> HashMap<i32, String> {
        order.iter().map(|k| (map[k].uid, k.clone())).collect()
    };
    let (ok, nk) = (uid_keys(&old_order, &old_map), uid_keys(&new_order, &new_map));
    let mut c = Comparison {
        matched_by: match by {
            MatchBy::ActivityId => "activity_id",
            MatchBy::Name => "name",
            _ => "uid",
        }
        .into(),
        old_finish: old.tasks.iter().filter_map(|t| t.finish).max().or(old.finish),
        new_finish: new.tasks.iter().filter_map(|t| t.finish).max().or(new.finish),
        finish_slip_days: None,
        old_tasks: old_order.len(),
        new_tasks: new_order.len(),
        added: vec![],
        removed: vec![],
        changed: vec![],
        unchanged: 0,
        coverage: MatchCoverage::default(),
    };
    if by == MatchBy::ActivityId {
        for (plan, order, map) in [("old", &old_order, &old_map), ("new", &new_order, &new_map)] {
            for k in order.iter().filter(|k| k.starts_with("uid:")) {
                let t = map[k];
                c.coverage.fallback_tasks.push(FallbackTask { plan: plan.into(), key: k.clone(), id: t.id, name: t.name.clone() });
                if plan == "old" {
                    c.coverage.old_uid_fallback += 1;
                } else {
                    c.coverage.new_uid_fallback += 1;
                }
            }
        }
    }
    c.finish_slip_days = slip_days_cal(&cal, c.old_finish, c.new_finish);
    for k in &old_order {
        if !new_map.contains_key(k) {
            c.removed.push(tref(k, old_map[k]));
        }
    }
    for k in &new_order {
        let n = new_map[k];
        let Some(o) = old_map.get(k) else {
            c.added.push(tref(k, n));
            continue;
        };
        match by {
            MatchBy::ActivityId if !k.starts_with("uid:") => c.coverage.by_activity_id += 1,
            MatchBy::Name => c.coverage.by_name += 1,
            _ => {
                c.coverage.by_uid += 1;
                if !o.name.trim().eq_ignore_ascii_case(n.name.trim()) {
                    c.coverage.name_mismatches.push(NameMismatch { key: k.clone(), old_name: o.name.clone(), new_name: n.name.clone() });
                }
            }
        }
        let mut ch = Vec::new();
        let mut push = |field: &str, old: String, new: String, delta: Option<f64>| {
            if old != new {
                ch.push(FieldChange { field: field.into(), old, new, delta });
            }
        };
        if by != MatchBy::Name {
            push("name", o.name.clone(), n.name.clone(), None);
        }
        push("start", fmt_dt(&o.start), fmt_dt(&n.start), slip_days_cal(&cal, o.start, n.start).map(|d| d as f64));
        push("finish", fmt_dt(&o.finish), fmt_dt(&n.finish), slip_days_cal(&cal, o.finish, n.finish).map(|d| d as f64));
        let r = |h: f64| (h * 100.0).round() / 100.0;
        push(
            "duration_hours",
            r(o.duration_hours).to_string(),
            r(n.duration_hours).to_string(),
            Some(r(n.duration_hours - o.duration_hours)),
        );
        push(
            "percent_complete",
            r(o.percent_complete).to_string(),
            r(n.percent_complete).to_string(),
            Some(r(n.percent_complete - o.percent_complete)),
        );
        push("milestone", o.milestone.to_string(), n.milestone.to_string(), None);
        let (lo, ln) = (links(o, &ok), links(n, &nk));
        if lo != ln {
            let rem: Vec<_> = lo.difference(&ln).cloned().collect();
            let add: Vec<_> = ln.difference(&lo).cloned().collect();
            ch.push(FieldChange { field: "predecessors".into(), old: rem.join("; "), new: add.join("; "), delta: None });
        }
        if ch.is_empty() {
            c.unchanged += 1;
        } else {
            c.changed.push(TaskChange { key: k.clone(), id: n.id, name: n.name.clone(), changes: ch });
        }
    }
    c
}

/// Plain-text report.
pub fn to_text(c: &Comparison) -> String {
    let mut s = String::new();
    s += &format!(
        "Compared {} → {} tasks (matched by {}). Project finish {} → {}{}\n",
        c.old_tasks,
        c.new_tasks,
        c.matched_by,
        fmt_dt(&c.old_finish),
        fmt_dt(&c.new_finish),
        c.finish_slip_days.map(|d| format!(" ({d:+} working days)")).unwrap_or_default()
    );
    s += &format!("added {}, removed {}, changed {}, unchanged {}\n", c.added.len(), c.removed.len(), c.changed.len(), c.unchanged);
    s += &format!("{}\n", c.coverage.summary(&c.matched_by));
    for w in [c.coverage.warning(), c.coverage.name_warning()].into_iter().flatten() {
        s += &format!("{w}\n");
    }
    for t in &c.added {
        s += &format!("+ [{}] {}  ({} → {})\n", t.key, t.name, fmt_dt(&t.start), fmt_dt(&t.finish));
    }
    for t in &c.removed {
        s += &format!("- [{}] {}\n", t.key, t.name);
    }
    for t in &c.changed {
        s += &format!("~ [{}] {}\n", t.key, t.name);
        for f in &t.changes {
            if f.field == "predecessors" {
                s += &format!("    predecessors: removed [{}] added [{}]\n", f.old, f.new);
            } else {
                let d = match (f.field.as_str(), f.delta) {
                    ("start" | "finish", Some(d)) => format!("  ({d:+} wd)"),
                    (_, Some(d)) if d != 0.0 => format!("  ({d:+})"),
                    _ => String::new(),
                };
                s += &format!("    {}: {} → {}{d}\n", f.field, f.old, f.new);
            }
        }
    }
    s
}

/// Self-contained HTML report.
pub fn to_html(c: &Comparison, old_name: &str, new_name: &str) -> String {
    let mut rows = String::new();
    for t in &c.added {
        rows += &format!(
            "<tr class=\"add\"><td>added</td><td>{}</td><td>{}</td><td colspan=\"3\">{} → {}</td></tr>\n",
            esc(&t.key),
            esc(&t.name),
            fmt_dt(&t.start),
            fmt_dt(&t.finish)
        );
    }
    for t in &c.removed {
        rows +=
            &format!("<tr class=\"rem\"><td>removed</td><td>{}</td><td>{}</td><td colspan=\"3\"></td></tr>\n", esc(&t.key), esc(&t.name));
    }
    for t in &c.changed {
        for (i, f) in t.changes.iter().enumerate() {
            let (a, b) = if i == 0 { (esc(&t.key), esc(&t.name)) } else { (String::new(), String::new()) };
            let d = f.delta.filter(|d| *d != 0.0).map(|d| format!(" ({d:+})")).unwrap_or_default();
            let late = f.delta.is_some_and(|d| d > 0.0) && (f.field == "finish" || f.field == "start");
            rows += &format!(
                "<tr class=\"chg{}\"><td>{}</td><td>{a}</td><td>{b}</td><td>{}</td><td>{}</td><td>{}{d}</td></tr>\n",
                if late { " late" } else { "" },
                if i == 0 { "changed" } else { "" },
                esc(&f.field),
                esc(&f.old),
                esc(&f.new)
            );
        }
    }
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>Schedule comparison</title><style>body{{font-family:Segoe UI,Arial,sans-serif;margin:16px}}table{{border-collapse:collapse;font-size:13px}}td,th{{border:1px solid #ddd;padding:3px 8px;text-align:left}}th{{background:#f5f5f5}}.add td:first-child{{color:#2e7d32}}.rem td:first-child{{color:#c62828}}.late td:last-child{{color:#c62828}}.kpi{{display:inline-block;margin:0 18px 12px 0}}.kpi b{{font-size:20px;display:block}}.warn{{color:#b45309;font-weight:bold}}</style></head><body>\n<h1>Schedule comparison</h1><p>{} → {} (matched by {})</p>\n<p class=\"cov\">{}</p>{}\n<div class=\"kpi\"><b>{}</b>finish slip (working days)</div><div class=\"kpi\"><b>{}</b>added</div><div class=\"kpi\"><b>{}</b>removed</div><div class=\"kpi\"><b>{}</b>changed</div><div class=\"kpi\"><b>{}</b>unchanged</div>\n<p>Project finish: {} → {}</p>\n<table><tr><th>Change</th><th>Key</th><th>Task</th><th>Field</th><th>Old</th><th>New</th></tr>\n{rows}</table></body></html>\n",
        esc(old_name),
        esc(new_name),
        c.matched_by,
        esc(&c.coverage.summary(&c.matched_by)),
        [c.coverage.warning(), c.coverage.name_warning()]
            .into_iter()
            .flatten()
            .map(|w| format!("<p class=\"warn\">{}</p>", esc(&w)))
            .collect::<String>(),
        c.finish_slip_days.map(|d| format!("{d:+}")).unwrap_or_else(|| "–".into()),
        c.added.len(),
        c.removed.len(),
        c.changed.len(),
        c.unchanged,
        fmt_dt(&c.old_finish),
        fmt_dt(&c.new_finish)
    )
}
