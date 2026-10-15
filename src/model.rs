//! Engine-neutral project model shared by all readers and writers.
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde::Serialize;

/// A dependency on another task.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Pred {
    /// Unique ID of the predecessor (stable, used for linking in MSPDI).
    pub predecessor_uid: i32,
    /// Visible row ID of the predecessor as shown in Microsoft Project (if known).
    pub predecessor_id: Option<i32>,
    /// FS, SS, FF or SF.
    pub link_type: String,
    /// Lag in working hours (negative = lead).
    pub lag_hours: f64,
    /// Lag as displayed in the source units, e.g. `2d`.
    pub lag_display: String,
}

/// Microsoft Project constraint types (numeric values match MSPDI `ConstraintType`).
pub const CONSTRAINT_NAMES: [&str; 8] = ["ASAP", "ALAP", "MSO", "MFO", "SNET", "SNLT", "FNET", "FNLT"];

#[derive(Serialize, Clone, Debug, Default)]
pub struct TaskRec {
    pub uid: i32,
    pub id: Option<i32>,
    pub name: String,
    pub outline_level: i32,
    pub wbs: Option<String>,
    pub start: Option<NaiveDateTime>,
    pub finish: Option<NaiveDateTime>,
    pub duration_hours: f64,
    pub percent_complete: f64,
    pub milestone: bool,
    pub summary: bool,
    pub critical: bool,
    /// True when the task is manually scheduled.
    pub manual: bool,
    pub predecessors: Vec<Pred>,
    pub baseline_start: Option<NaiveDateTime>,
    pub baseline_finish: Option<NaiveDateTime>,
    pub total_slack_hours: Option<f64>,
    pub notes: Option<String>,
    /// MSPDI constraint type 0..=7 (see [`CONSTRAINT_NAMES`]); `None` = ASAP.
    pub constraint_type: Option<i32>,
    pub constraint_date: Option<NaiveDateTime>,
    pub deadline: Option<NaiveDateTime>,
    pub actual_start: Option<NaiveDateTime>,
    pub actual_finish: Option<NaiveDateTime>,
    pub cost: Option<f64>,
    pub fixed_cost: Option<f64>,
    pub actual_cost: Option<f64>,
    /// Task calendar UID, if the task has its own calendar.
    pub calendar_uid: Option<i32>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct ResourceRec {
    pub uid: i32,
    pub id: Option<i32>,
    pub name: String,
    /// Work, Material or Cost.
    pub rtype: String,
    pub initials: Option<String>,
    /// Max units in percent (100 = one full-time person).
    pub max_units: Option<f64>,
    pub email: Option<String>,
    pub group: Option<String>,
    /// Standard rate per hour.
    pub standard_rate: Option<f64>,
    pub cost_per_use: Option<f64>,
    pub cost: Option<f64>,
    pub calendar_uid: Option<i32>,
    pub notes: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct AssignmentRec {
    pub uid: i32,
    pub task_uid: i32,
    pub resource_uid: i32,
    /// Units in percent.
    pub units: f64,
    pub work_hours: f64,
    pub start: Option<NaiveDateTime>,
    pub finish: Option<NaiveDateTime>,
    pub cost: Option<f64>,
}

/// Working hours of one weekday. `working == None` means "inherit from base calendar".
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct DayRec {
    pub working: Option<bool>,
    pub ranges: Vec<(NaiveTime, NaiveTime)>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ExceptionRec {
    pub name: Option<String>,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub working: bool,
    pub ranges: Vec<(NaiveTime, NaiveTime)>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct CalendarRec {
    pub uid: i32,
    pub name: String,
    pub base_uid: Option<i32>,
    /// Index 0 = Sunday ... 6 = Saturday (MSPDI DayType 1..=7).
    pub days: [DayRec; 7],
    pub exceptions: Vec<ExceptionRec>,
}

impl CalendarRec {
    /// The default Microsoft Project "Standard" calendar: Mon–Fri 08:00–12:00, 13:00–17:00.
    pub fn standard(uid: i32) -> Self {
        let t = |h| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        let work = DayRec { working: Some(true), ranges: vec![(t(8), t(12)), (t(13), t(17))] };
        let off = DayRec { working: Some(false), ranges: vec![] };
        CalendarRec {
            uid,
            name: "Standard".into(),
            base_uid: None,
            days: [off.clone(), work.clone(), work.clone(), work.clone(), work.clone(), work, off],
            exceptions: vec![],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub format: String,
    pub title: Option<String>,
    pub name: Option<String>,
    pub author: Option<String>,
    pub manager: Option<String>,
    pub company: Option<String>,
    pub start: Option<NaiveDateTime>,
    pub finish: Option<NaiveDateTime>,
    pub minutes_per_day: i32,
    pub minutes_per_week: i32,
    pub default_calendar_uid: Option<i32>,
    pub calendars: Vec<CalendarRec>,
    pub tasks: Vec<TaskRec>,
    pub resources: Vec<ResourceRec>,
    pub assignments: Vec<AssignmentRec>,
}

#[derive(Serialize)]
pub struct Summary {
    pub format: String,
    pub title: Option<String>,
    pub name: Option<String>,
    pub author: Option<String>,
    pub manager: Option<String>,
    pub company: Option<String>,
    pub start: Option<NaiveDateTime>,
    pub finish: Option<NaiveDateTime>,
    pub task_count: usize,
    pub summary_tasks: usize,
    pub milestones: usize,
    pub completed_tasks: usize,
    pub critical_tasks: usize,
    pub resource_count: usize,
    pub assignment_count: usize,
    pub relation_count: usize,
    pub calendar_count: usize,
    pub calendar_exception_count: usize,
    pub tasks_with_baseline: usize,
    pub total_cost: Option<f64>,
    pub overall_percent_complete: f64,
    pub critical_path: Vec<String>,
}

impl Plan {
    pub fn summary(&self) -> Summary {
        let leaf: Vec<&TaskRec> = self.tasks.iter().filter(|t| !t.summary).collect();
        let (mut wsum, mut w) = (0.0, 0.0);
        for t in &leaf {
            wsum += t.percent_complete * t.duration_hours;
            w += t.duration_hours;
        }
        let costs: Vec<f64> = leaf.iter().filter_map(|t| t.cost).collect();
        Summary {
            format: self.format.clone(),
            title: self.title.clone(),
            name: self.name.clone(),
            author: self.author.clone(),
            manager: self.manager.clone(),
            company: self.company.clone(),
            start: self.start,
            finish: self.finish,
            task_count: self.tasks.len(),
            summary_tasks: self.tasks.iter().filter(|t| t.summary).count(),
            milestones: self.tasks.iter().filter(|t| t.milestone).count(),
            completed_tasks: self.tasks.iter().filter(|t| t.percent_complete >= 100.0).count(),
            critical_tasks: leaf.iter().filter(|t| t.critical).count(),
            resource_count: self.resources.len(),
            assignment_count: self.assignments.len(),
            relation_count: self.tasks.iter().map(|t| t.predecessors.len()).sum(),
            calendar_count: self.calendars.len(),
            calendar_exception_count: self.calendars.iter().map(|c| c.exceptions.len()).sum(),
            tasks_with_baseline: self.tasks.iter().filter(|t| t.baseline_start.is_some()).count(),
            total_cost: if costs.is_empty() { None } else { Some(costs.iter().sum()) },
            overall_percent_complete: if w > 0.0 { wsum / w } else { 0.0 },
            critical_path: leaf
                .iter()
                .filter(|t| t.critical)
                .take(50)
                .map(|t| format!("[ID {}] {} ({} → {})", t.id.unwrap_or(t.uid), t.name, fmt_dt(&t.start), fmt_dt(&t.finish)))
                .collect(),
        }
    }

    /// Fill `Pred::predecessor_id` from the tasks' visible IDs.
    pub fn resolve_pred_ids(&mut self) {
        let map: std::collections::HashMap<i32, Option<i32>> = self.tasks.iter().map(|t| (t.uid, t.id)).collect();
        for t in &mut self.tasks {
            for p in &mut t.predecessors {
                if p.predecessor_id.is_none() {
                    p.predecessor_id = map.get(&p.predecessor_uid).copied().flatten();
                }
            }
        }
    }
}

pub fn fmt_dt(d: &Option<NaiveDateTime>) -> String {
    d.map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

/// Format predecessors like Microsoft Project, e.g. `3;5SS+2d`. `by_id` uses visible row IDs
/// (falls back to UID when an ID is unknown); otherwise UIDs.
pub fn preds_str(p: &[Pred], by_id: bool) -> String {
    p.iter()
        .map(|p| {
            let n = if by_id { p.predecessor_id.unwrap_or(p.predecessor_uid) } else { p.predecessor_uid };
            let mut s = n.to_string();
            if p.link_type != "FS" || p.lag_hours != 0.0 {
                s += &p.link_type;
            }
            if p.lag_hours != 0.0 {
                s += &format!("{}{}", if p.lag_hours > 0.0 { "+" } else { "" }, p.lag_display);
            }
            s
        })
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pred(uid: i32, id: Option<i32>, ty: &str, lag: f64, disp: &str) -> Pred {
        Pred { predecessor_uid: uid, predecessor_id: id, link_type: ty.into(), lag_hours: lag, lag_display: disp.into() }
    }

    #[test]
    fn preds_formatting() {
        let p = vec![pred(7, Some(3), "FS", 0.0, ""), pred(9, Some(5), "SS", 16.0, "2d"), pred(4, None, "FF", -8.0, "-1d")];
        assert_eq!(preds_str(&p, true), "3;5SS+2d;4FF-1d");
        assert_eq!(preds_str(&p, false), "7;9SS+2d;4FF-1d");
    }

    #[test]
    fn standard_calendar_shape() {
        let c = CalendarRec::standard(1);
        assert_eq!(c.days[0].working, Some(false));
        assert_eq!(c.days[1].ranges.len(), 2);
        assert_eq!(c.days[6].working, Some(false));
    }
}
