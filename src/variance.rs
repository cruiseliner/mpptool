//! Baseline variance and earned value.
use crate::calendar::WorkCalendar;
use crate::model::{Plan, TaskRec};
use crate::workdays::slip_days_cal;
use chrono::NaiveDateTime;
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct TaskVariance {
    pub uid: i32,
    pub id: Option<i32>,
    pub activity_id: Option<String>,
    pub name: String,
    pub outline_level: i32,
    pub summary: bool,
    pub baseline_start: Option<NaiveDateTime>,
    pub baseline_finish: Option<NaiveDateTime>,
    /// Actual start when known, else the current forecast start.
    pub start: Option<NaiveDateTime>,
    /// Actual finish when known, else the current forecast finish.
    pub finish: Option<NaiveDateTime>,
    /// Working days late (+) or early (−) versus the baseline.
    pub start_slip_days: Option<i64>,
    pub finish_slip_days: Option<i64>,
    pub duration_variance_hours: Option<f64>,
    pub percent_complete: f64,
    /// Planned value (BCWS) at the status date.
    pub pv: Option<f64>,
    /// Earned value (BCWP).
    pub ev: Option<f64>,
    /// Actual cost (ACWP).
    pub ac: Option<f64>,
    pub bac: Option<f64>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct EarnedValue {
    pub bac: f64,
    pub pv: f64,
    pub ev: f64,
    pub ac: f64,
    /// Schedule variance EV − PV.
    pub sv: f64,
    /// Cost variance EV − AC.
    pub cv: f64,
    /// Schedule performance index EV / PV.
    pub spi: Option<f64>,
    /// Cost performance index EV / AC.
    pub cpi: Option<f64>,
    /// Estimate at completion BAC / CPI.
    pub eac: Option<f64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct VarianceReport {
    pub status_date: Option<NaiveDateTime>,
    pub tasks_with_baseline: usize,
    pub late_finishes: usize,
    pub max_finish_slip_days: Option<i64>,
    /// Project finish slip (latest finish vs latest baseline finish), working days.
    pub project_finish_slip_days: Option<i64>,
    /// `None` when the plan has no baseline cost.
    pub earned_value: Option<EarnedValue>,
    pub tasks: Vec<TaskVariance>,
}

/// Fraction of the baseline that should be done at `at`: linear over the baseline's working days on the
/// Standard (Mon–Fri) calendar, counting the status date itself as worked.
pub fn planned_fraction(t: &TaskRec, at: NaiveDateTime) -> Option<f64> {
    let cal = crate::calendar::CalendarSpec::default().resolve(2000).expect("standard calendar");
    planned_fraction_cal(&cal, t, at)
}

/// [`planned_fraction`] on calendar `cal` (the plan's project calendar).
pub fn planned_fraction_cal(cal: &WorkCalendar, t: &TaskRec, at: NaiveDateTime) -> Option<f64> {
    let working_days_between = |a, b| cal.working_days_between(a, b);
    let (s, f) = (t.baseline_start?, t.baseline_finish?);
    Some(if at >= f {
        1.0
    } else if at < s {
        0.0
    } else {
        let total = working_days_between(s.date(), f.date()) + 1;
        let done = working_days_between(s.date(), at.date()) + 1;
        (done as f64 / total.max(1) as f64).clamp(0.0, 1.0)
    })
}

/// Build the variance report. `status` falls back to the plan's status date.
pub fn variance(plan: &Plan, status: Option<NaiveDateTime>) -> VarianceReport {
    let status = status.or(plan.status_date);
    let cal = crate::calendar::plan_calendar(plan);
    let has_cost = plan.tasks.iter().any(|t| !t.summary && t.baseline_finish.is_some() && t.baseline_cost.is_some_and(|c| c > 0.0));
    let mut ev_tot = EarnedValue::default();
    let mut tasks = Vec::new();
    for t in plan.tasks.iter().filter(|t| t.baseline_start.is_some() || t.baseline_finish.is_some()) {
        let start = t.actual_start.or(t.start);
        let finish = t.actual_finish.or(t.finish);
        let bac = t.baseline_cost.filter(|_| has_cost);
        let pv = match (bac, status) {
            (Some(b), Some(sd)) => planned_fraction_cal(&cal, t, sd).map(|f| b * f),
            _ => None,
        };
        let ev = bac.map(|b| b * t.percent_complete.clamp(0.0, 100.0) / 100.0);
        let ac = if has_cost { Some(t.actual_cost.unwrap_or(0.0)) } else { None };
        if !t.summary && has_cost {
            ev_tot.bac += bac.unwrap_or(0.0);
            ev_tot.pv += pv.unwrap_or(0.0);
            ev_tot.ev += ev.unwrap_or(0.0);
            ev_tot.ac += ac.unwrap_or(0.0);
        }
        tasks.push(TaskVariance {
            uid: t.uid,
            id: t.id,
            activity_id: t.activity_id.clone(),
            name: t.name.clone(),
            outline_level: t.outline_level,
            summary: t.summary,
            baseline_start: t.baseline_start,
            baseline_finish: t.baseline_finish,
            start,
            finish,
            start_slip_days: slip_days_cal(&cal, t.baseline_start, start),
            finish_slip_days: slip_days_cal(&cal, t.baseline_finish, finish),
            duration_variance_hours: t.baseline_duration_hours.map(|b| ((t.duration_hours - b) * 100.0).round() / 100.0),
            percent_complete: t.percent_complete,
            pv,
            ev,
            ac,
            bac,
        });
    }
    let leaf = || tasks.iter().filter(|t| !t.summary);
    let earned_value = has_cost.then(|| {
        let e = &mut ev_tot;
        e.sv = e.ev - e.pv;
        e.cv = e.ev - e.ac;
        e.spi = (e.pv > 0.0 && status.is_some()).then(|| e.ev / e.pv);
        e.cpi = (e.ac > 0.0).then(|| e.ev / e.ac);
        e.eac = e.cpi.filter(|c| *c > 0.0).map(|c| e.bac / c);
        if status.is_none() {
            e.sv = 0.0;
        }
        ev_tot.clone()
    });
    let latest = |f: &dyn Fn(&TaskRec) -> Option<NaiveDateTime>| plan.tasks.iter().filter(|t| !t.summary).filter_map(f).max();
    VarianceReport {
        status_date: status,
        tasks_with_baseline: leaf().count(),
        late_finishes: leaf().filter(|t| t.finish_slip_days.is_some_and(|d| d > 0)).count(),
        max_finish_slip_days: leaf().filter_map(|t| t.finish_slip_days).max(),
        project_finish_slip_days: slip_days_cal(&cal, latest(&|t| t.baseline_finish), latest(&|t| t.actual_finish.or(t.finish))),
        earned_value,
        tasks,
    }
}
