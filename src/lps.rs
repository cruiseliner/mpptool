//! Last Planner System® metrics from CSV: Percent Plan Complete (PPC) per week and per responsible party, reasons for
//! variance (Pareto), the constraint log (open, overdue, resolved on time, days to resolve) and an optional make-ready
//! check of a schedule's lookahead window against open constraints.
//!
//! PPC follows the Lean Construction Institute definition: assignments completed as promised in the weekly work plan
//! divided by assignments committed that week.
use chrono::NaiveDate;
use serde::Serialize;
use std::collections::BTreeMap;

/// Header-indexed CSV records (lower-case headers, trimmed fields, `#` comments).
pub(crate) struct Table {
    pub hdr: Vec<String>,
    pub recs: Vec<(usize, Vec<String>)>,
}

impl Table {
    pub fn parse(text: &str, what: &str) -> Result<Table, String> {
        let mut rdr = csv::ReaderBuilder::new().flexible(true).trim(csv::Trim::All).comment(Some(b'#')).from_reader(text.as_bytes());
        let hdr: Vec<String> = rdr.headers().map_err(|e| format!("{what} CSV header: {e}"))?.iter().map(|h| h.to_lowercase()).collect();
        let mut recs = vec![];
        for (k, rec) in rdr.records().enumerate() {
            let rec = rec.map_err(|e| format!("{what} line {}: {e}", k + 2))?;
            if rec.iter().all(|f| f.trim().is_empty()) {
                continue;
            }
            recs.push((k + 2, rec.iter().map(|f| f.trim().to_string()).collect()));
        }
        Ok(Table { hdr, recs })
    }
    pub fn col(&self, names: &[&str]) -> Option<usize> {
        names.iter().find_map(|n| self.hdr.iter().position(|h| h == n))
    }
}

pub(crate) fn get(r: &[String], c: Option<usize>) -> &str {
    c.and_then(|c| r.get(c)).map(|s| s.as_str()).unwrap_or("")
}

pub(crate) fn yes(v: &str) -> Option<bool> {
    match v.to_lowercase().as_str() {
        "y" | "yes" | "true" | "1" | "x" | "done" | "complete" | "completed" => Some(true),
        "n" | "no" | "false" | "0" | "" | "not done" | "incomplete" => Some(false),
        _ => None,
    }
}

pub(crate) fn date(v: &str, line: usize, what: &str) -> Result<Option<NaiveDate>, String> {
    if v.is_empty() {
        return Ok(None);
    }
    NaiveDate::parse_from_str(v, "%Y-%m-%d").map(Some).map_err(|_| format!("line {line}: bad {what} '{v}' (want YYYY-MM-DD)"))
}

#[derive(Debug, Serialize)]
pub struct WeekPpc {
    pub week: String,
    pub committed: usize,
    pub completed: usize,
    pub ppc: f64,
}

#[derive(Debug, Serialize)]
pub struct PartyPpc {
    pub responsible: String,
    pub committed: usize,
    pub completed: usize,
    pub ppc: f64,
}

#[derive(Debug, Serialize)]
pub struct Reason {
    pub reason: String,
    pub count: usize,
    /// Share of all missed commitments.
    pub share: f64,
}

#[derive(Debug, Serialize, Clone)]
pub struct Constraint {
    pub id: String,
    pub activity: String,
    pub description: String,
    pub category: String,
    pub owner: String,
    pub raised: Option<NaiveDate>,
    pub need_by: Option<NaiveDate>,
    pub resolved: Option<NaiveDate>,
    /// Open and past its need-by date at the status date.
    pub overdue: bool,
}

#[derive(Debug, Serialize, Default)]
pub struct ConstraintSummary {
    pub total: usize,
    pub open: usize,
    pub overdue: usize,
    pub resolved: usize,
    /// Resolved on or before the need-by date, as a share of resolved constraints with a need-by date.
    pub resolved_on_time: Option<f64>,
    pub mean_days_to_resolve: Option<f64>,
    /// Open constraints per owner (champion) and per category.
    pub open_by_owner: BTreeMap<String, usize>,
    pub open_by_category: BTreeMap<String, usize>,
    pub overdue_items: Vec<Constraint>,
}

/// A schedule activity that starts inside the lookahead window.
#[derive(Debug, Serialize)]
pub struct LookaheadTask {
    pub activity_id: Option<String>,
    pub name: String,
    pub early_start: NaiveDate,
    /// Open constraint IDs on this activity.
    pub open_constraints: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Lookahead {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub tasks: usize,
    pub ready: usize,
    /// Share of lookahead activities with no open constraint (made ready).
    pub ready_share: f64,
    pub not_ready: Vec<LookaheadTask>,
}

#[derive(Debug, Serialize)]
pub struct LpsReport {
    pub status_date: NaiveDate,
    pub weeks: Vec<WeekPpc>,
    pub overall_ppc: Option<f64>,
    /// Mean PPC of the last four weeks.
    pub rolling_4wk_ppc: Option<f64>,
    /// Last week's PPC minus the week before (percentage points).
    pub trend_points: Option<f64>,
    pub by_responsible: Vec<PartyPpc>,
    pub reasons: Vec<Reason>,
    /// Missed commitments with no reason recorded.
    pub missed_without_reason: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<ConstraintSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookahead: Option<Lookahead>,
}

/// Weekly work plan CSV: `week` (date or label), `activity` (or `task`/`assignment`), `responsible` (or
/// `trade`/`company`/`last_planner`), optional `committed` (default yes), `completed` (yes/no), `reason` (reason for
/// variance when not completed). Rows with `committed = no` are listed for information but excluded from PPC.
/// PPC by week, PPC by responsible party, reasons for variance, missed commitments with no reason.
pub type PpcParts = (Vec<WeekPpc>, Vec<PartyPpc>, Vec<Reason>, usize);

pub fn ppc(text: &str) -> Result<PpcParts, String> {
    let t = Table::parse(text, "work plan")?;
    let cw = t.col(&["week", "week_of", "week_start"]).ok_or("work plan CSV needs a 'week' column")?;
    let cc = t.col(&["completed", "done", "complete"]).ok_or("work plan CSV needs a 'completed' column")?;
    let ccm = t.col(&["committed", "commitment"]);
    let cr = t.col(&["responsible", "trade", "company", "last_planner", "owner"]);
    let crs = t.col(&["reason", "reason_for_variance", "rfv", "variance_reason"]);
    let mut weeks: Vec<(String, usize, usize)> = vec![];
    let mut party: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    let mut no_reason = 0;
    for (line, r) in &t.recs {
        let w = get(r, Some(cw)).to_string();
        if w.is_empty() {
            return Err(format!("work plan line {line}: missing week"));
        }
        let committed = match ccm {
            Some(_) if get(r, ccm).is_empty() => true,
            Some(_) => yes(get(r, ccm)).ok_or(format!("work plan line {line}: bad committed '{}'", get(r, ccm)))?,
            None => true,
        };
        if !committed {
            continue;
        }
        let done = yes(get(r, Some(cc))).ok_or(format!("work plan line {line}: bad completed '{}' (yes/no)", get(r, Some(cc))))?;
        let i = match weeks.iter().position(|x| x.0 == w) {
            Some(i) => i,
            None => {
                weeks.push((w, 0, 0));
                weeks.len() - 1
            }
        };
        weeks[i].1 += 1;
        let p = party.entry(Some(get(r, cr)).filter(|s| !s.is_empty()).unwrap_or("(unassigned)").to_string()).or_default();
        p.0 += 1;
        if done {
            weeks[i].2 += 1;
            p.1 += 1;
        } else {
            match get(r, crs) {
                "" => no_reason += 1,
                x => *reasons.entry(normalize_reason(x)).or_default() += 1,
            }
        }
    }
    // Weeks given as dates sort chronologically; labels keep file order.
    if weeks.iter().all(|w| NaiveDate::parse_from_str(&w.0, "%Y-%m-%d").is_ok()) {
        weeks.sort_by(|a, b| a.0.cmp(&b.0));
    }
    let pct = |a: usize, b: usize| if b == 0 { 0.0 } else { a as f64 / b as f64 };
    let missed: usize = reasons.values().sum::<usize>() + no_reason;
    let mut rs: Vec<Reason> =
        reasons.into_iter().map(|(reason, count)| Reason { reason, count, share: pct(count, missed.max(1)) }).collect();
    rs.sort_by(|a, b| b.count.cmp(&a.count).then(a.reason.cmp(&b.reason)));
    let mut ps: Vec<PartyPpc> =
        party.into_iter().map(|(responsible, (c, d))| PartyPpc { responsible, committed: c, completed: d, ppc: pct(d, c) }).collect();
    ps.sort_by(|a, b| a.ppc.total_cmp(&b.ppc).then(a.responsible.cmp(&b.responsible)));
    Ok((weeks.into_iter().map(|(week, c, d)| WeekPpc { week, committed: c, completed: d, ppc: pct(d, c) }).collect(), ps, rs, no_reason))
}

/// Map free-text reasons onto the usual LPS variance categories when they clearly match; otherwise keep the text.
fn normalize_reason(s: &str) -> String {
    let l = s.to_lowercase();
    let cats: &[(&str, &[&str])] = &[
        ("Prerequisite work", &["prereq", "predecessor", "previous trade", "prior work"]),
        ("Design / RFI", &["rfi", "design", "drawing"]),
        ("Submittal / approval", &["submittal", "approval", "approve"]),
        ("Materials", &["material", "delivery", "supply"]),
        ("Labor / crew", &["labor", "labour", "crew", "manpower", "staff"]),
        ("Equipment", &["equipment", "crane", "lift", "tool"]),
        ("Weather", &["weather", "rain", "snow", "wind", "heat", "cold"]),
        ("Inspection / permit", &["inspect", "permit", "ahj"]),
        ("Space / access", &["space", "access", "congest", "laydown"]),
        ("Owner change", &["owner", "change order", "scope change"]),
        ("Underestimated effort", &["underestim", "took longer", "productivity"]),
    ];
    for (c, keys) in cats {
        if keys.iter().any(|k| l.contains(k)) {
            return c.to_string();
        }
    }
    s.trim().to_string()
}

/// Constraint log CSV: `id`, `activity_id` (or `activity`/`task`), `description`, `category` (or `type`), `owner`
/// (or `champion`/`responsible`), `raised`, `need_by` (or `due`), `resolved` (date), optional `status` (open/closed;
/// a resolved date closes the item).
pub fn parse_constraints(text: &str, status: NaiveDate) -> Result<Vec<Constraint>, String> {
    let t = Table::parse(text, "constraint log")?;
    let ci = t.col(&["id", "constraint_id"]);
    let ca = t.col(&["activity_id", "activity", "task", "task_id"]);
    let cd = t.col(&["description", "constraint", "name"]);
    let ck = t.col(&["category", "type", "kind"]);
    let co = t.col(&["owner", "champion", "responsible"]);
    let (cr, cn, cx, cs) = (
        t.col(&["raised", "identified", "date_raised"]),
        t.col(&["need_by", "due", "need_date", "due_date"]),
        t.col(&["resolved", "removed", "closed", "date_resolved"]),
        t.col(&["status"]),
    );
    if ca.is_none() && cd.is_none() {
        return Err("constraint log needs an 'activity_id' (or activity) or 'description' column".into());
    }
    let mut out = vec![];
    for (k, (line, r)) in t.recs.iter().enumerate() {
        let raised = date(get(r, cr), *line, "raised")?;
        let need_by = date(get(r, cn), *line, "need_by")?;
        let mut resolved = date(get(r, cx), *line, "resolved")?;
        let st = get(r, cs).to_lowercase();
        if resolved.is_none() && matches!(st.as_str(), "closed" | "resolved" | "removed" | "done") {
            resolved = Some(status);
        }
        if let (Some(a), Some(b)) = (raised, resolved)
            && b < a
        {
            return Err(format!("constraint line {line}: resolved before raised"));
        }
        out.push(Constraint {
            id: Some(get(r, ci).to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("C{}", k + 1)),
            activity: get(r, ca).to_string(),
            description: get(r, cd).to_string(),
            category: Some(get(r, ck).to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "(none)".into()),
            owner: Some(get(r, co).to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "(no owner)".into()),
            raised,
            need_by,
            resolved,
            overdue: resolved.is_none() && need_by.is_some_and(|d| d < status),
        });
    }
    Ok(out)
}

pub fn summarize_constraints(cs: &[Constraint]) -> ConstraintSummary {
    let mut s = ConstraintSummary { total: cs.len(), ..Default::default() };
    let (mut on_time, mut with_need, mut days, mut nd) = (0usize, 0usize, 0i64, 0usize);
    for c in cs {
        match c.resolved {
            Some(r) => {
                s.resolved += 1;
                if let Some(n) = c.need_by {
                    with_need += 1;
                    if r <= n {
                        on_time += 1;
                    }
                }
                if let Some(a) = c.raised {
                    days += (r - a).num_days();
                    nd += 1;
                }
            }
            None => {
                s.open += 1;
                *s.open_by_owner.entry(c.owner.clone()).or_default() += 1;
                *s.open_by_category.entry(c.category.clone()).or_default() += 1;
                if c.overdue {
                    s.overdue += 1;
                    s.overdue_items.push(c.clone());
                }
            }
        }
    }
    s.overdue_items.sort_by_key(|c| c.need_by);
    s.resolved_on_time = (with_need > 0).then(|| on_time as f64 / with_need as f64);
    s.mean_days_to_resolve = (nd > 0).then(|| days as f64 / nd as f64);
    s
}

/// Lookahead make-ready check: activities of `rows` whose early start falls in `from..=to`, against open constraints
/// keyed by activity ID (or exact name).
pub fn lookahead(
    rows: &[crate::from_csv::Row],
    early: &[Option<chrono::NaiveDateTime>],
    summary: &[bool],
    cs: &[Constraint],
    from: NaiveDate,
    to: NaiveDate,
) -> Lookahead {
    let mut tasks = 0;
    let mut not_ready = vec![];
    for (i, r) in rows.iter().enumerate() {
        let Some(es) = early.get(i).copied().flatten() else { continue };
        if summary.get(i).copied().unwrap_or(false) || r.is_complete() || es.date() < from || es.date() > to {
            continue;
        }
        tasks += 1;
        let open: Vec<String> = cs
            .iter()
            .filter(|c| c.resolved.is_none())
            .filter(|c| {
                let a = c.activity.trim();
                !a.is_empty()
                    && (r.activity_id.as_deref().is_some_and(|x| x.trim().eq_ignore_ascii_case(a)) || r.name.trim().eq_ignore_ascii_case(a))
            })
            .map(|c| c.id.clone())
            .collect();
        if !open.is_empty() {
            not_ready.push(LookaheadTask {
                activity_id: r.activity_id.clone(),
                name: r.name.clone(),
                early_start: es.date(),
                open_constraints: open,
            });
        }
    }
    not_ready.sort_by_key(|t| t.early_start);
    let ready = tasks - not_ready.len();
    Lookahead { from, to, tasks, ready, ready_share: if tasks == 0 { 1.0 } else { ready as f64 / tasks as f64 }, not_ready }
}

pub fn report(plan_text: &str, constraints: Option<&str>, status: NaiveDate) -> Result<LpsReport, String> {
    let (weeks, by_responsible, reasons, missed_without_reason) = ppc(plan_text)?;
    let (c, d): (usize, usize) = weeks.iter().fold((0, 0), |a, w| (a.0 + w.committed, a.1 + w.completed));
    let last4: Vec<f64> = weeks.iter().rev().take(4).map(|w| w.ppc).collect();
    let cons = constraints.map(|t| parse_constraints(t, status)).transpose()?;
    Ok(LpsReport {
        status_date: status,
        overall_ppc: (c > 0).then(|| d as f64 / c as f64),
        rolling_4wk_ppc: (!last4.is_empty()).then(|| last4.iter().sum::<f64>() / last4.len() as f64),
        trend_points: (weeks.len() >= 2).then(|| (weeks[weeks.len() - 1].ppc - weeks[weeks.len() - 2].ppc) * 100.0),
        weeks,
        by_responsible,
        reasons,
        missed_without_reason,
        constraints: cons.as_deref().map(summarize_constraints),
        lookahead: None,
    })
}

pub fn render_text(r: &LpsReport) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "Last Planner metrics (status date {})", r.status_date);
    let _ = writeln!(s, "PPC by week (completed as promised / committed):");
    for w in &r.weeks {
        let bar = "#".repeat((w.ppc * 20.0).round() as usize);
        let _ = writeln!(s, "  {:<12} {:>3}/{:<3} {:>5.0}%  {bar}", w.week, w.completed, w.committed, w.ppc * 100.0);
    }
    if let Some(p) = r.overall_ppc {
        let _ = write!(s, "Overall PPC {:.0}%", p * 100.0);
        if let Some(x) = r.rolling_4wk_ppc {
            let _ = write!(s, "; last 4 weeks {:.0}%", x * 100.0);
        }
        if let Some(t) = r.trend_points {
            let _ = write!(s, "; last week {t:+.0} points");
        }
        let _ = writeln!(s, "  (LCI: high-performing teams average 75-90%)");
    }
    if !r.by_responsible.is_empty() {
        let _ = writeln!(s, "PPC by responsible party (lowest first):");
        for p in &r.by_responsible {
            let _ = writeln!(
                s,
                "  {:<28} {:>3}/{:<3} {:>5.0}%",
                p.responsible.chars().take(28).collect::<String>(),
                p.completed,
                p.committed,
                p.ppc * 100.0
            );
        }
    }
    if !r.reasons.is_empty() || r.missed_without_reason > 0 {
        let _ = writeln!(s, "Reasons for variance (Pareto):");
        let mut cum = 0.0;
        for x in &r.reasons {
            cum += x.share;
            let _ = writeln!(
                s,
                "  {:<32} {:>3}  {:>4.0}%  (cum {:>3.0}%)",
                x.reason.chars().take(32).collect::<String>(),
                x.count,
                x.share * 100.0,
                cum * 100.0
            );
        }
        if r.missed_without_reason > 0 {
            let _ = writeln!(s, "  WARNING: {} missed commitment(s) have no reason recorded", r.missed_without_reason);
        }
    }
    if let Some(c) = &r.constraints {
        let _ = writeln!(s, "Constraint log: {} total, {} open ({} overdue), {} resolved", c.total, c.open, c.overdue, c.resolved);
        if let Some(x) = c.resolved_on_time {
            let _ = write!(s, "  resolved by need-by date: {:.0}%", x * 100.0);
        }
        if let Some(x) = c.mean_days_to_resolve {
            let _ = write!(s, "  mean days to resolve: {x:.1}");
        }
        if c.resolved_on_time.is_some() || c.mean_days_to_resolve.is_some() {
            s.push('\n');
        }
        if !c.open_by_owner.is_empty() {
            let v: Vec<String> = c.open_by_owner.iter().map(|(k, v)| format!("{k} {v}")).collect();
            let _ = writeln!(s, "  open by owner: {}", v.join(", "));
            let v: Vec<String> = c.open_by_category.iter().map(|(k, v)| format!("{k} {v}")).collect();
            let _ = writeln!(s, "  open by category: {}", v.join(", "));
        }
        for o in &c.overdue_items {
            let _ = writeln!(
                s,
                "  OVERDUE {:<6} need by {} owner {:<16} {} {}",
                o.id,
                o.need_by.map(|d| d.to_string()).unwrap_or_default(),
                o.owner,
                o.activity,
                o.description
            );
        }
    }
    if let Some(l) = &r.lookahead {
        let _ = writeln!(
            s,
            "Lookahead {}..{}: {} activities start, {} made ready ({:.0}%), {} blocked by open constraints",
            l.from,
            l.to,
            l.tasks,
            l.ready,
            l.ready_share * 100.0,
            l.not_ready.len()
        );
        for t in &l.not_ready {
            let _ = writeln!(
                s,
                "  {} {:<10} {:<36} open: {}",
                t.early_start,
                t.activity_id.clone().unwrap_or_default(),
                t.name.chars().take(36).collect::<String>(),
                t.open_constraints.join(", ")
            );
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn ppc_reasons_and_constraints() {
        let wwp = "week,activity,responsible,committed,completed,reason\n\
            2026-11-09,Layout,GC,yes,yes,\n2026-11-09,Rough-in,Elec,yes,no,Material late\n\
            2026-11-02,Demo,GC,yes,yes,\n2026-11-02,Saw cut,GC,yes,yes,\n2026-11-09,Pipe,Plumb,no,no,\n\
            2026-11-09,Frame,GC,,no,RFI 12 open\n";
        let r = report(wwp, None, d("2026-11-13")).unwrap();
        assert_eq!(r.weeks[0].week, "2026-11-02", "date weeks sort chronologically");
        assert_eq!((r.weeks[1].committed, r.weeks[1].completed), (3, 1), "uncommitted rows excluded");
        assert!((r.overall_ppc.unwrap() - 0.6).abs() < 1e-9);
        assert!((r.trend_points.unwrap() - (100.0 / 3.0 - 100.0)).abs() < 1e-6);
        assert_eq!(r.reasons.len(), 2);
        assert!(r.reasons.iter().any(|x| x.reason == "Materials") && r.reasons.iter().any(|x| x.reason == "Design / RFI"));
        assert!(report("week,activity,completed\nw1,A,maybe\n", None, d("2026-11-13")).is_err());
        let cl = "id,activity_id,description,category,owner,raised,need_by,resolved\n\
            C1,A20,Submittal approval,submittal,Arch,2026-10-20,2026-11-01,2026-10-30\n\
            C2,A30,RFI 12,design,Arch,2026-10-25,2026-11-05,\n\
            C3,A40,Lift on site,equipment,GC,2026-11-01,2026-11-20,\n\
            C4,A50,Permit,permit,PM,2026-10-01,2026-10-15,2026-10-20\n";
        let cs = parse_constraints(cl, d("2026-11-13")).unwrap();
        let s = summarize_constraints(&cs);
        assert_eq!((s.total, s.open, s.overdue, s.resolved), (4, 2, 1, 2));
        assert_eq!(s.overdue_items[0].id, "C2");
        assert_eq!(s.resolved_on_time, Some(0.5));
        assert_eq!(s.mean_days_to_resolve, Some((10.0 + 19.0) / 2.0));
        let rows = crate::from_csv::parse_rows(
            "activity_id,name,start,duration_days,predecessors\nA20,Install,2026-11-16,2,\nA30,Frame,2026-11-16,2,\nA40,Lift,2026-12-30,1,\n",
        )
        .unwrap();
        let es: Vec<_> = rows.iter().map(|r| r.start.map(|x| x.and_hms_opt(8, 0, 0).unwrap())).collect();
        let l = lookahead(&rows, &es, &[false; 3], &cs, d("2026-11-13"), d("2026-12-25"));
        assert_eq!((l.tasks, l.ready), (2, 1));
        assert_eq!(l.not_ready[0].open_constraints, vec!["C2"]);
        assert!(render_text(&r).contains("Overall PPC 60%"));
    }
}
