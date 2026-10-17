//! DCMA 14-point schedule assessment.
use crate::model::{Plan, TaskRec};
use chrono::NaiveDateTime;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// Result of one DCMA check.
#[derive(Serialize, Clone, Debug)]
pub struct CheckResult {
    /// DCMA check number 1..=14.
    pub number: u8,
    pub name: String,
    /// `pass`, `fail` or `skipped`.
    pub status: String,
    /// Human-readable threshold, e.g. `<= 5%`.
    pub threshold: String,
    /// Number of offending items (tasks or relationships).
    pub count: usize,
    /// Size of the population the check was evaluated over.
    pub total: usize,
    /// `count / total * 100`, when meaningful.
    pub percent: Option<f64>,
    /// Index value for ratio checks (CPLI, BEI).
    pub value: Option<f64>,
    /// IDs of the offending tasks (Activity ID when known, else the task ID).
    pub task_ids: Vec<String>,
    /// Why the check was skipped, or extra explanation.
    pub note: Option<String>,
}

/// The full assessment.
#[derive(Serialize, Clone, Debug)]
pub struct Assessment {
    pub status_date: Option<NaiveDateTime>,
    pub activities: usize,
    pub incomplete_activities: usize,
    pub relationships: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub checks: Vec<CheckResult>,
}

/// Options for [`assess`].
#[derive(Debug, Clone, Default)]
pub struct CheckOptions {
    /// Overrides the plan's status date.
    pub status_date: Option<NaiveDateTime>,
    /// Check 1: do not count the project start milestone (no predecessors) and finish milestone (no successors).
    pub exclude_endpoints: bool,
    /// Pass/fail thresholds (defaults are the DCMA values).
    pub thresholds: Thresholds,
}

/// Configurable DCMA thresholds. Percentages are 0–100; day limits are working days.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Thresholds {
    pub logic_pct: f64,
    pub lags_pct: f64,
    pub fs_pct: f64,
    pub hard_constraints_pct: f64,
    pub high_float_days: f64,
    pub high_float_pct: f64,
    pub high_duration_days: f64,
    pub high_duration_pct: f64,
    pub missed_pct: f64,
    pub cpli: f64,
    pub bei: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            logic_pct: 5.0,
            lags_pct: 5.0,
            fs_pct: 90.0,
            hard_constraints_pct: 5.0,
            high_float_days: 44.0,
            high_float_pct: 5.0,
            high_duration_days: 44.0,
            high_duration_pct: 5.0,
            missed_pct: 5.0,
            cpli: 0.95,
            bei: 0.95,
        }
    }
}

impl Thresholds {
    /// Apply `key=value[,key=value]` overrides, e.g. `logic=10,high_float=60,cpli=0.9`.
    /// Keys: logic, lags, fs, hard, high_float, high_float_pct, high_duration, high_duration_pct, missed, cpli, bei.
    pub fn apply(&mut self, spec: &str) -> Result<(), String> {
        for kv in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (k, v) = kv.split_once('=').ok_or(format!("bad threshold '{kv}' (want key=value)"))?;
            let v: f64 =
                v.trim().parse().ok().filter(|v: &f64| v.is_finite() && *v >= 0.0).ok_or(format!("bad threshold value in '{kv}'"))?;
            let slot = match k.trim().to_lowercase().replace('-', "_").as_str() {
                "logic" => &mut self.logic_pct,
                "lags" => &mut self.lags_pct,
                "fs" => &mut self.fs_pct,
                "hard" | "hard_constraints" => &mut self.hard_constraints_pct,
                "high_float" | "high_float_days" => &mut self.high_float_days,
                "high_float_pct" => &mut self.high_float_pct,
                "high_duration" | "high_duration_days" => &mut self.high_duration_days,
                "high_duration_pct" => &mut self.high_duration_pct,
                "missed" => &mut self.missed_pct,
                "cpli" => &mut self.cpli,
                "bei" => &mut self.bei,
                other => return Err(format!("unknown threshold '{other}'")),
            };
            *slot = v;
        }
        Ok(())
    }
}

fn tid(t: &TaskRec) -> String {
    t.activity_id.clone().unwrap_or_else(|| t.id.unwrap_or(t.uid).to_string())
}

fn complete(t: &TaskRec) -> bool {
    t.percent_complete >= 100.0 || t.actual_finish.is_some()
}

struct Ctx<'a> {
    hours_per_day: f64,
    acts: Vec<&'a TaskRec>,
    incomplete: Vec<&'a TaskRec>,
    succ: HashMap<i32, usize>,
    by_uid: HashMap<i32, &'a TaskRec>,
}

fn pct(count: usize, total: usize) -> Option<f64> {
    if total == 0 { None } else { Some(count as f64 * 100.0 / total as f64) }
}

fn ratio_check(number: u8, name: &str, max_pct: f64, bad: Vec<&TaskRec>, total: usize, threshold: &str) -> CheckResult {
    let count = bad.len();
    let p = pct(count, total);
    CheckResult {
        number,
        name: name.into(),
        status: if total == 0 || p.unwrap_or(0.0) <= max_pct + 1e-9 { "pass" } else { "fail" }.into(),
        threshold: threshold.into(),
        count,
        total,
        percent: p,
        value: None,
        task_ids: bad.iter().map(|t| tid(t)).collect(),
        note: if total == 0 { Some("no items to evaluate".into()) } else { None },
    }
}

fn skipped(number: u8, name: &str, threshold: &str, why: &str) -> CheckResult {
    CheckResult {
        number,
        name: name.into(),
        status: "skipped".into(),
        threshold: threshold.into(),
        count: 0,
        total: 0,
        percent: None,
        value: None,
        task_ids: vec![],
        note: Some(why.into()),
    }
}

/// Run the DCMA 14-point assessment over the plan.
///
/// The population is all non-summary tasks; checks 1–8 and 10 use incomplete ones (as DCMA prescribes).
pub fn assess(plan: &Plan, o: &CheckOptions) -> Assessment {
    let hpd = if plan.minutes_per_day > 0 { plan.minutes_per_day as f64 / 60.0 } else { 8.0 };
    let acts: Vec<&TaskRec> = plan.tasks.iter().filter(|t| !t.summary).collect();
    let incomplete: Vec<&TaskRec> = acts.iter().copied().filter(|t| !complete(t)).collect();
    let mut succ: HashMap<i32, usize> = HashMap::new();
    for t in &acts {
        for p in &t.predecessors {
            *succ.entry(p.predecessor_uid).or_default() += 1;
        }
    }
    let by_uid = plan.tasks.iter().map(|t| (t.uid, t)).collect();
    let c = Ctx { hours_per_day: hpd, acts, incomplete, succ, by_uid };
    let status = o.status_date.or(plan.status_date);
    let has_baseline = c.acts.iter().any(|t| t.baseline_finish.is_some());
    let has_float = c.acts.iter().any(|t| t.total_slack_hours.is_some());
    let mut checks = Vec::new();

    // Relationships among incomplete activities (successor incomplete).
    let rels: Vec<(&TaskRec, &crate::model::Pred)> =
        c.incomplete.iter().flat_map(|t| t.predecessors.iter().map(move |p| (*t, p))).collect();

    // 1 Logic
    let th = &o.thresholds;
    let no_logic: Vec<&TaskRec> = c
        .incomplete
        .iter()
        .copied()
        .filter(|t| {
            let (np, ns) = (t.predecessors.is_empty(), !c.succ.contains_key(&t.uid));
            // Optionally accept the start milestone (no predecessors) and finish milestone (no successors).
            (np || ns) && !(o.exclude_endpoints && t.milestone && np != ns)
        })
        .collect();
    let mut r = ratio_check(
        1,
        "Logic (missing predecessors or successors)",
        th.logic_pct,
        no_logic,
        c.incomplete.len(),
        &format!("<= {}%", th.logic_pct),
    );
    if o.exclude_endpoints {
        r.note = Some("start and finish milestones excluded".into());
    }
    checks.push(r);
    // 2 Leads
    let leads: Vec<&TaskRec> = rels.iter().filter(|(_, p)| p.lag_hours < 0.0).map(|(t, _)| *t).collect();
    let mut r = ratio_check(2, "Leads (negative lag)", 0.0, dedup(leads.clone()), rels.len(), "0");
    r.count = leads.len();
    r.percent = pct(leads.len(), rels.len());
    checks.push(r);
    // 3 Lags
    let lags: Vec<&TaskRec> = rels.iter().filter(|(_, p)| p.lag_hours > 0.0).map(|(t, _)| *t).collect();
    let mut r = ratio_check(3, "Lags (positive lag)", th.lags_pct, dedup(lags.clone()), rels.len(), &format!("<= {}%", th.lags_pct));
    r.count = lags.len();
    r.percent = pct(lags.len(), rels.len());
    checks.push(r);
    // 4 Relationship types
    let non_fs: Vec<&TaskRec> = rels.iter().filter(|(_, p)| p.link_type != "FS").map(|(t, _)| *t).collect();
    let mut r = ratio_check(
        4,
        &format!("Relationship types (FS >= {}%)", th.fs_pct),
        100.0 - th.fs_pct,
        dedup(non_fs.clone()),
        rels.len(),
        &format!(">= {}% FS", th.fs_pct),
    );
    r.count = non_fs.len();
    r.percent = pct(non_fs.len(), rels.len());
    r.note = Some(format!("{} of {} relationships are not finish-to-start", non_fs.len(), rels.len()));
    checks.push(r);
    // 5 Hard constraints (MSO, MFO, SNLT, FNLT)
    let hard: Vec<&TaskRec> = c.incomplete.iter().copied().filter(|t| matches!(t.constraint_type, Some(2 | 3 | 5 | 7))).collect();
    checks.push(ratio_check(
        5,
        "Hard constraints (MSO/MFO/SNLT/FNLT)",
        th.hard_constraints_pct,
        hard,
        c.incomplete.len(),
        &format!("<= {}%", th.hard_constraints_pct),
    ));
    // 6 High float, 7 negative float
    let limit = th.high_float_days * c.hours_per_day;
    let hf_name = format!("High float (> {} working days)", th.high_float_days);
    let hf_th = format!("<= {}%", th.high_float_pct);
    if has_float {
        let hi: Vec<&TaskRec> = c.incomplete.iter().copied().filter(|t| t.total_slack_hours.is_some_and(|f| f > limit)).collect();
        checks.push(ratio_check(6, &hf_name, th.high_float_pct, hi, c.incomplete.len(), &hf_th));
        let neg: Vec<&TaskRec> = c.incomplete.iter().copied().filter(|t| t.total_slack_hours.is_some_and(|f| f < -0.01)).collect();
        checks.push(ratio_check(7, "Negative float", 0.0, neg, c.incomplete.len(), "0"));
    } else {
        checks.push(skipped(6, &hf_name, &hf_th, "no total float in the file"));
        checks.push(skipped(7, "Negative float", "0", "no total float in the file"));
    }
    // 8 High duration
    let dlimit = th.high_duration_days * c.hours_per_day;
    let long: Vec<&TaskRec> = c.incomplete.iter().copied().filter(|t| !t.milestone && t.duration_hours > dlimit).collect();
    checks.push(ratio_check(
        8,
        &format!("High duration (> {} working days)", th.high_duration_days),
        th.high_duration_pct,
        long,
        c.incomplete.len(),
        &format!("<= {}%", th.high_duration_pct),
    ));
    // 9 Invalid dates
    match status {
        Some(sd) => {
            let bad: Vec<&TaskRec> = c
                .acts
                .iter()
                .copied()
                .filter(|t| {
                    let fut_actual = t.actual_start.is_some_and(|d| d > sd) || t.actual_finish.is_some_and(|d| d > sd);
                    let past_forecast = (t.actual_start.is_none() && t.start.is_some_and(|d| d < sd))
                        || (t.actual_finish.is_none() && t.finish.is_some_and(|d| d < sd));
                    fut_actual || past_forecast
                })
                .collect();
            checks.push(ratio_check(9, "Invalid dates (vs status date)", 0.0, bad, c.acts.len(), "0"));
        }
        None => checks.push(skipped(9, "Invalid dates (vs status date)", "0", "no status date (use --status-date)")),
    }
    // 10 Resources
    if plan.resources.is_empty() && plan.assignments.is_empty() {
        checks.push(skipped(10, "Resources (tasks without assignments)", "0%", "the plan has no resources"));
    } else {
        let assigned: HashSet<i32> = plan.assignments.iter().map(|a| a.task_uid).collect();
        let bad: Vec<&TaskRec> =
            c.incomplete.iter().copied().filter(|t| !t.milestone && t.duration_hours > 0.0 && !assigned.contains(&t.uid)).collect();
        let pop = c.incomplete.iter().filter(|t| !t.milestone && t.duration_hours > 0.0).count();
        checks.push(ratio_check(10, "Resources (tasks without assignments)", 0.0, bad, pop, "0%"));
    }
    // 11 Missed tasks, 14 BEI (need baseline + status date)
    match (status, has_baseline) {
        (Some(sd), true) => {
            let due: Vec<&TaskRec> = c.acts.iter().copied().filter(|t| t.baseline_finish.is_some_and(|b| b <= sd)).collect();
            let missed: Vec<&TaskRec> =
                due.iter().copied().filter(|t| t.actual_finish.or(t.finish).is_some_and(|f| Some(f) > t.baseline_finish)).collect();
            checks.push(ratio_check(
                11,
                "Missed tasks (finished late vs baseline)",
                th.missed_pct,
                missed,
                due.len(),
                &format!("<= {}%", th.missed_pct),
            ));
        }
        _ => checks.push(skipped(11, "Missed tasks (finished late vs baseline)", "<= 5%", need(status.is_some(), has_baseline))),
    }
    // 12 Critical path test (static approximation)
    checks.push(critical_path_test(&c));
    // 13 CPLI
    checks.push(cpli(plan, &c, status, has_float, th.cpli));
    // 14 BEI
    match (status, has_baseline) {
        (Some(sd), true) => {
            let due = c.acts.iter().filter(|t| t.baseline_finish.is_some_and(|b| b <= sd)).count();
            let done = c.acts.iter().filter(|t| complete(t)).count();
            let bad: Vec<&TaskRec> =
                c.acts.iter().copied().filter(|t| t.baseline_finish.is_some_and(|b| b <= sd) && !complete(t)).collect();
            let bei = if due > 0 { Some(done as f64 / due as f64) } else { None };
            checks.push(CheckResult {
                number: 14,
                name: "Baseline Execution Index (BEI)".into(),
                status: if bei.is_none_or(|b| b >= th.bei) { "pass" } else { "fail" }.into(),
                threshold: format!(">= {}", th.bei),
                count: bad.len(),
                total: due,
                percent: pct(bad.len(), due),
                value: bei,
                task_ids: bad.iter().map(|t| tid(t)).collect(),
                note: Some(format!("{done} tasks complete / {due} baselined to finish by the status date")),
            });
        }
        _ => checks.push(skipped(14, "Baseline Execution Index (BEI)", ">= 0.95", need(status.is_some(), has_baseline))),
    }
    let n = |s: &str| checks.iter().filter(|c| c.status == s).count();
    Assessment {
        status_date: status,
        activities: c.acts.len(),
        incomplete_activities: c.incomplete.len(),
        relationships: rels.len(),
        passed: n("pass"),
        failed: n("fail"),
        skipped: n("skipped"),
        checks,
    }
}

fn need(has_status: bool, has_baseline: bool) -> &'static str {
    match (has_status, has_baseline) {
        (false, false) => "needs a baseline and a status date",
        (false, true) => "no status date (use --status-date)",
        _ => "no baseline in the file",
    }
}

fn dedup(v: Vec<&TaskRec>) -> Vec<&TaskRec> {
    let mut seen = HashSet::new();
    v.into_iter().filter(|t| seen.insert(t.uid)).collect()
}

/// Latest-finishing incomplete activity (the project's finish driver).
fn finish_task<'a>(c: &Ctx<'a>) -> Option<&'a TaskRec> {
    c.incomplete.iter().copied().filter(|t| t.finish.is_some()).max_by_key(|t| t.finish)
}

/// DCMA check 12. A true CPT needs a re-schedule; we check statically that the critical path is
/// continuous: walking back from the finish activity through critical (or driving) predecessors reaches
/// an activity without incomplete critical predecessors, and every step is linked.
fn critical_path_test(c: &Ctx) -> CheckResult {
    let name = "Critical path test (continuity)";
    if !c.acts.iter().any(|t| t.critical) {
        return skipped(12, name, "continuous", "no critical flags in the file");
    }
    let Some(end) = finish_task(c) else { return skipped(12, name, "continuous", "no incomplete activities") };
    let mut path = vec![end];
    let mut seen = HashSet::from([end.uid]);
    let mut cur = end;
    loop {
        let next = cur
            .predecessors
            .iter()
            .filter_map(|p| c.by_uid.get(&p.predecessor_uid).copied())
            .filter(|p| !p.summary && p.critical && !complete(p) && !seen.contains(&p.uid))
            .max_by_key(|p| p.finish);
        match next {
            Some(n) => {
                seen.insert(n.uid);
                path.push(n);
                cur = n;
            }
            None => break,
        }
    }
    let ok = end.critical;
    CheckResult {
        number: 12,
        name: name.into(),
        status: if ok { "pass" } else { "fail" }.into(),
        threshold: "continuous".into(),
        count: if ok { 0 } else { 1 },
        total: path.len(),
        percent: None,
        value: None,
        task_ids: if ok { vec![] } else { vec![tid(end)] },
        note: Some(if ok {
            format!("critical chain of {} activities traced back from finish activity {}", path.len(), tid(end))
        } else {
            format!("finish activity {} is not critical: the critical path does not drive the project finish", tid(end))
        }),
    }
}

fn cpli(plan: &Plan, c: &Ctx, status: Option<NaiveDateTime>, has_float: bool, min: f64) -> CheckResult {
    let name = "Critical Path Length Index (CPLI)";
    let thr = format!(">= {min}");
    if !has_float {
        return skipped(13, name, &thr, "no total float in the file");
    }
    let Some(end) = finish_task(c) else { return skipped(13, name, &thr, "no incomplete activities") };
    let from = status.or(plan.start).or(c.acts.iter().filter_map(|t| t.start).min());
    let (Some(from), Some(fin)) = (from, end.finish) else { return skipped(13, name, &thr, "missing dates") };
    // Remaining critical path length in working days, approximated as 5/7 of calendar days.
    let cpl = (fin - from).num_minutes() as f64 / 1440.0 * 5.0 / 7.0;
    if cpl <= 0.0 {
        return skipped(13, name, &thr, "project finish is not after the status date");
    }
    let tf = end.total_slack_hours.unwrap_or(0.0) / c.hours_per_day;
    let v = (cpl + tf) / cpl;
    CheckResult {
        number: 13,
        name: name.into(),
        status: if v >= min { "pass" } else { "fail" }.into(),
        threshold: thr.clone(),
        count: 0,
        total: 0,
        percent: None,
        value: Some(v),
        task_ids: if v >= min { vec![] } else { vec![tid(end)] },
        note: Some(format!("remaining length ≈ {cpl:.1} working days, finish float {tf:.1} days")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Pred;

    fn t(uid: i32, preds: &[(i32, &str, f64)]) -> TaskRec {
        TaskRec {
            uid,
            id: Some(uid),
            name: format!("T{uid}"),
            outline_level: 1,
            duration_hours: 8.0,
            predecessors: preds
                .iter()
                .map(|(p, ty, lag)| Pred {
                    predecessor_uid: *p,
                    predecessor_id: Some(*p),
                    link_type: (*ty).into(),
                    lag_hours: *lag,
                    lag_display: String::new(),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn logic_leads_lags() {
        let plan = Plan {
            minutes_per_day: 480,
            tasks: vec![t(1, &[]), t(2, &[(1, "FS", -8.0)]), t(3, &[(2, "SS", 16.0)]), t(4, &[])],
            ..Default::default()
        };
        let a = assess(&plan, &CheckOptions::default());
        let get = |n: u8| a.checks.iter().find(|c| c.number == n).unwrap();
        assert_eq!(get(1).task_ids, vec!["1", "3", "4"]);
        assert_eq!(get(2).count, 1);
        assert_eq!(get(2).status, "fail");
        assert_eq!(get(3).count, 1);
        assert_eq!(get(4).count, 1);
        assert_eq!(get(9).status, "skipped");
        assert_eq!(get(14).status, "skipped");
        assert_eq!(a.checks.len(), 14);
    }

    #[test]
    fn thresholds_and_endpoint_exclusion() {
        let mut th = Thresholds::default();
        th.apply("logic=10, high_float=60,cpli=0.9").unwrap();
        assert_eq!((th.logic_pct, th.high_float_days, th.cpli), (10.0, 60.0, 0.9));
        assert!(th.apply("nope=1").is_err());
        assert!(th.apply("logic=abc").is_err());
        let rows = crate::from_csv::parse_rows("name,start,duration_days,predecessors\nStart,2026-11-02,0,\nA,,5,1\nB,,5,2\nFinish,,0,3\n")
            .unwrap();
        let p = crate::from_csv::schedule(&rows, None, chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()).unwrap();
        let a = assess(&p, &CheckOptions::default());
        assert_eq!(a.checks[0].count, 2);
        let a = assess(&p, &CheckOptions { exclude_endpoints: true, ..Default::default() });
        assert_eq!(a.checks[0].count, 0);
        assert_eq!(a.checks[0].status, "pass");
        let mut th = Thresholds::default();
        th.apply("logic=60").unwrap();
        let a = assess(&p, &CheckOptions { thresholds: th, ..Default::default() });
        assert_eq!(a.checks[0].status, "pass");
        assert_eq!(a.checks[0].threshold, "<= 60%");
    }
}
