//! Roll up every readable plan in a folder into one dashboard.
use crate::gantt::esc;
use crate::model::fmt_dt;
use crate::{Engine, ReadOptions};
use chrono::NaiveDateTime;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone, Debug)]
pub struct PlanRow {
    pub file: String,
    /// Project id within the file (Primavera multi-project files), else `None`.
    pub project: Option<String>,
    pub name: String,
    pub format: String,
    pub start: Option<NaiveDateTime>,
    pub finish: Option<NaiveDateTime>,
    pub status_date: Option<NaiveDateTime>,
    pub tasks: usize,
    pub percent_complete: f64,
    pub critical_tasks: usize,
    /// Project finish vs baseline finish in working days (positive = late); `None` without a baseline.
    pub finish_slip_days: Option<i64>,
    /// % of applicable DCMA checks passed (0–100).
    pub health_score: f64,
    pub checks_failed: usize,
    pub rag: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Skipped {
    pub file: String,
    pub error: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Portfolio {
    pub folder: String,
    pub plans: Vec<PlanRow>,
    pub skipped: Vec<Skipped>,
}

const EXTS: [&str; 4] = ["mpp", "mpx", "xml", "xer"];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTS.iter().any(|x| x.eq_ignore_ascii_case(e))) {
            out.push(p);
        }
    }
}

fn row(file: &str, project: Option<String>, plan: &crate::model::Plan) -> PlanRow {
    let s = plan.summary();
    let a = crate::check::assess(plan, &Default::default());
    let v = crate::variance::variance(plan, None);
    let applicable = a.passed + a.failed;
    let health = if applicable > 0 { (a.passed as f64 * 1000.0 / applicable as f64).round() / 10.0 } else { 100.0 };
    let slip = v.project_finish_slip_days;
    let rag = if slip.is_some_and(|d| d > 10) || health < 50.0 {
        "red"
    } else if slip.is_some_and(|d| d > 0) || health < 75.0 {
        "amber"
    } else {
        "green"
    };
    PlanRow {
        file: file.into(),
        project,
        name: s.title.clone().or(s.name.clone()).unwrap_or_else(|| file.into()),
        format: s.format,
        start: plan.start.or(plan.tasks.iter().filter_map(|t| t.start).min()),
        finish: plan.finish.or(plan.tasks.iter().filter_map(|t| t.finish).max()),
        status_date: plan.status_date,
        tasks: plan.tasks.iter().filter(|t| !t.summary).count(),
        percent_complete: (s.overall_percent_complete * 10.0).round() / 10.0,
        critical_tasks: s.critical_tasks,
        finish_slip_days: slip,
        health_score: health,
        checks_failed: a.failed,
        rag: rag.into(),
    }
}

/// Read every `.mpp/.mpx/.xml/.xer` file below `folder` (every project of multi-project Primavera files).
/// Files that cannot be read as plans are listed in `skipped`.
pub fn build(folder: &Path, opts: &ReadOptions, engine: Engine) -> Portfolio {
    let mut files = Vec::new();
    walk(folder, &mut files);
    let mut p = Portfolio { folder: folder.display().to_string(), plans: vec![], skipped: vec![] };
    for f in files {
        let rel = f.strip_prefix(folder).unwrap_or(&f).display().to_string().replace('\\', "/");
        if crate::is_primavera(&f) {
            match crate::read_p6::parse_file(&f) {
                Ok(d) => {
                    for pr in &d.projects {
                        match crate::read_p6::to_plan(&d, Some(&pr.key)) {
                            Ok(plan) => p.plans.push(row(&rel, Some(pr.short_name.clone()), &plan)),
                            Err(e) => p.skipped.push(Skipped { file: rel.clone(), error: e }),
                        }
                    }
                    if d.projects.is_empty() {
                        p.skipped.push(Skipped { file: rel, error: "no projects in file".into() });
                    }
                }
                Err(e) => p.skipped.push(Skipped { file: rel, error: e }),
            }
            continue;
        }
        match std::panic::catch_unwind(|| crate::read(&f, opts, engine)) {
            Ok(Ok(plan)) => p.plans.push(row(&rel, None, &plan)),
            Ok(Err(e)) => p.skipped.push(Skipped { file: rel, error: e }),
            Err(_) => p.skipped.push(Skipped { file: rel, error: "reader panicked".into() }),
        }
    }
    p
}

/// Self-contained HTML dashboard.
pub fn to_html(p: &Portfolio) -> String {
    let n = |r: &str| p.plans.iter().filter(|x| x.rag == r).count();
    let mut rows = String::new();
    for r in &p.plans {
        rows += &format!(
            "<tr><td><span class=\"rag {rag}\">{RAG}</span></td><td class=\"l\"><b>{}</b><div class=\"f\">{}{}</div></td><td>{}</td><td>{}</td><td>{}</td><td><div class=\"bar\"><i style=\"width:{:.0}%\"></i></div>{:.0}%</td><td class=\"{}\">{}</td><td>{}</td><td>{}</td><td><span class=\"score {}\">{:.0}</span></td></tr>\n",
            esc(&r.name),
            esc(&r.file),
            r.project.as_ref().map(|x| format!(" · {}", esc(x))).unwrap_or_default(),
            esc(&r.format),
            fmt_dt(&r.start).get(..10).unwrap_or(""),
            fmt_dt(&r.finish).get(..10).unwrap_or(""),
            r.percent_complete.clamp(0.0, 100.0),
            r.percent_complete,
            if r.finish_slip_days.is_some_and(|d| d > 0) { "late" } else { "" },
            r.finish_slip_days.map(|d| format!("{d:+}")).unwrap_or_else(|| "–".into()),
            r.critical_tasks,
            r.tasks,
            if r.health_score >= 75.0 {
                "g"
            } else if r.health_score >= 50.0 {
                "a"
            } else {
                "r"
            },
            r.health_score,
            rag = r.rag,
            RAG = r.rag.to_uppercase()
        );
    }
    let mut skipped = String::new();
    if !p.skipped.is_empty() {
        skipped += &format!("<details><summary>{} file(s) could not be read</summary><ul>", p.skipped.len());
        for s in &p.skipped {
            skipped += &format!("<li>{} — {}</li>", esc(&s.file), esc(&s.error));
        }
        skipped += "</ul></details>";
    }
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>Portfolio</title><style>\
body{{font-family:Segoe UI,Arial,sans-serif;margin:16px;background:#fafafa}}table{{border-collapse:collapse;background:#fff;font-size:13px}}td,th{{border-bottom:1px solid #e5e5e5;padding:5px 9px;text-align:right}}th{{background:#f0f0f0;position:sticky;top:0}}.l{{text-align:left}}.f{{color:#888;font-size:11px}}\
.rag{{font-size:10px;font-weight:bold;color:#fff;border-radius:3px;padding:1px 5px}}.rag.red{{background:#d32f2f}}.rag.amber{{background:#f9a825}}.rag.green{{background:#43a047}}.late{{color:#c62828;font-weight:bold}}\
.bar{{display:inline-block;width:60px;height:5px;background:#eee;margin-right:5px;vertical-align:middle}}.bar i{{display:block;height:5px;background:#1e88e5}}.score{{font-weight:bold}}.score.g{{color:#2e7d32}}.score.a{{color:#f57f17}}.score.r{{color:#c62828}}\
.kpi{{display:inline-block;margin:0 22px 14px 0}}.kpi b{{display:block;font-size:22px}}</style></head><body>\n<h1>Portfolio</h1><p>{folder}</p>\
<div class=\"kpi\"><b>{}</b>plans</div><div class=\"kpi\"><b style=\"color:#d32f2f\">{}</b>red</div><div class=\"kpi\"><b style=\"color:#f9a825\">{}</b>amber</div><div class=\"kpi\"><b style=\"color:#43a047\">{}</b>green</div>\n\
<table><tr><th>RAG</th><th class=\"l\">Plan</th><th>Format</th><th>Start</th><th>Finish</th><th>% complete</th><th>Slip (wd)</th><th>Critical</th><th>Tasks</th><th>Health</th></tr>\n{rows}</table>\n{skipped}\
<p class=\"f\">Slip = latest finish vs latest baseline finish in working days. Health = % of applicable DCMA 14-point checks passed. RAG: red if slip &gt; 10 wd or health &lt; 50; amber if any slip or health &lt; 75.</p></body></html>\n",
        p.plans.len(),
        n("red"),
        n("amber"),
        n("green"),
        folder = esc(&p.folder)
    )
}
