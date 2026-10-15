//! Alternative MPP14 (Project 2010–365) reader using the pure-Rust `mpxrust` crate.
//! Day-level dates only (start = 08:00, finish = 17:00); no calendars/baselines/critical flags.
use crate::model::*;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use std::path::Path;

fn d(s: &Option<String>, h: u32) -> Option<NaiveDateTime> {
    let s = s.as_deref()?;
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok().or_else(|| {
        NaiveDate::parse_from_str(&s[..s.len().min(10)], "%Y-%m-%d").ok().map(|d| d.and_time(NaiveTime::from_hms_opt(h, 0, 0).unwrap()))
    })
}

pub fn read(path: &Path) -> Result<Plan, String> {
    let pf = mpxrust::read_mpp(path).map_err(|e| format!("mpxrust failed to read {}: {e}", path.display()))?;
    let mpd = pf.properties.minutes_per_day.unwrap_or(480) as i32;
    let hpd = mpd as f64 / 60.0;
    let mut plan = Plan {
        format: "MPP14 (mpxrust)".into(),
        title: pf.properties.title.clone(),
        start: d(&pf.properties.start_date, 8),
        finish: d(&pf.properties.finish_date, 17),
        minutes_per_day: mpd,
        minutes_per_week: pf.properties.minutes_per_week.unwrap_or(2400) as i32,
        ..Default::default()
    };
    for t in &pf.tasks {
        plan.tasks.push(TaskRec {
            uid: t.uid as i32,
            id: t.id.map(|v| v as i32),
            name: t.name.clone().unwrap_or_default(),
            outline_level: t.outline_level.unwrap_or(1) as i32,
            wbs: t.wbs.clone(),
            start: d(&t.start_date, 8),
            finish: d(&t.finish_date, 17),
            duration_hours: t.duration_days.unwrap_or(0.0) * hpd,
            percent_complete: t.percent_complete.unwrap_or(0) as f64,
            milestone: t.is_milestone,
            summary: t.is_summary,
            notes: t.notes.clone(),
            predecessors: t
                .predecessors
                .iter()
                .map(|r| Pred {
                    predecessor_uid: r.predecessor_uid as i32,
                    link_type: match r.kind {
                        mpxrust::model::RelationType::FinishFinish => "FF",
                        mpxrust::model::RelationType::StartFinish => "SF",
                        mpxrust::model::RelationType::StartStart => "SS",
                        _ => "FS",
                    }
                    .into(),
                    predecessor_id: None,
                    lag_hours: r.lag_days * hpd,
                    lag_display: format!("{}d", r.lag_days),
                })
                .collect(),
            ..Default::default()
        });
    }
    for r in &pf.resources {
        plan.resources.push(ResourceRec {
            uid: r.uid as i32,
            id: Some(r.uid as i32),
            name: r.name.clone().unwrap_or_default(),
            rtype: "Work".into(),
            email: r.email.clone(),
            max_units: Some(100.0),
            ..Default::default()
        });
    }
    for (i, a) in pf.assignments.iter().enumerate() {
        plan.assignments.push(AssignmentRec {
            uid: i as i32 + 1,
            task_uid: a.task_uid as i32,
            resource_uid: a.resource_uid as i32,
            units: a.units.unwrap_or(1.0) * if a.units.unwrap_or(1.0) <= 10.0 { 100.0 } else { 1.0 },
            work_hours: a.work_hours.unwrap_or(0.0),
            start: None,
            finish: None,
            cost: None,
        });
    }
    plan.resolve_pred_ids();
    Ok(plan)
}
