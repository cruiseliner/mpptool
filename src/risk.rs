//! Monte Carlo schedule risk analysis (calendar-aware) for CSV plans and any schedule mpptool reads.
//!
//! Each run samples every leaf task's remaining duration from a PERT-beta distribution over (optimistic, most
//! likely, pessimistic). Most likely is the (remaining) duration; optimistic/pessimistic come from the CSV columns,
//! else from the task's uncertainty class (`labor`, `vendor`, `fixed`, …), else from `opt_factor`×M / `pess_factor`×M.
//! Discrete risk events from a RAID CSV fire with their probability and add one sampled impact to every linked task
//! (a common cause hits all of them in the same run). The network is re-scheduled with the same links, lags,
//! constraints, progress and working calendar as `from-csv`. The report gives finish percentiles, each task's
//! criticality index (share of runs on the driving path) and sensitivity (correlation of its duration with the
//! project finish), and each risk event's hit rate and mean finish impact.
use crate::calendar::CalendarSpec;
use crate::from_csv::{self, Network, Row, ScheduleOptions};
use chrono::{NaiveDate, NaiveDateTime};
use serde::Serialize;
use std::collections::HashMap;

/// Default (optimistic, pessimistic) factors of the built-in uncertainty classes, as multiples of the most likely
/// duration. These are planning defaults, not industry statistics: calibrate them against your own history.
pub const CLASSES: &[(&str, f64, f64, &str)] = &[
    ("labor", 0.85, 1.4, "self-performed or sub crew work, production-rate driven"),
    ("equipment", 0.9, 1.5, "equipment-driven work"),
    ("design", 0.8, 1.6, "design, engineering, reviews"),
    ("vendor", 0.95, 2.0, "vendor-quoted lead times, fabrication, delivery (long right tail)"),
    ("permit", 0.9, 2.0, "permits, AHJ reviews, utility work by others"),
    ("weather", 0.9, 1.8, "weather-exposed exterior work"),
    ("fixed", 1.0, 1.0, "fixed durations: concrete cure, contractual notice periods"),
];

#[derive(Debug, Clone)]
pub struct RiskOptions {
    pub runs: usize,
    pub seed: u64,
    pub opt_factor: f64,
    pub pess_factor: f64,
    /// Status date for plans with progress.
    pub status_date: Option<NaiveDate>,
    /// Discrete risk events (see [`parse_events`]).
    pub events: Vec<RiskEvent>,
    /// Overrides/additions to [`CLASSES`]: class → (optimistic factor, pessimistic factor).
    pub classes: HashMap<String, (f64, f64)>,
}

impl Default for RiskOptions {
    fn default() -> Self {
        RiskOptions {
            runs: 10_000,
            seed: 1,
            opt_factor: 0.8,
            pess_factor: 1.5,
            status_date: None,
            events: vec![],
            classes: CLASSES.iter().map(|c| (c.0.to_string(), (c.1, c.2))).collect(),
        }
    }
}

/// A discrete risk (RAID) event.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RiskEvent {
    pub id: String,
    pub name: String,
    /// 0–1.
    pub probability: f64,
    /// Impact in working days (min, most likely, max), added to each linked task when the event fires.
    pub min_days: f64,
    pub likely_days: f64,
    pub max_days: f64,
    /// Linked tasks: activity IDs or 1-based row numbers (`#3` forces a row number). A summary row applies to all
    /// its leaf tasks.
    pub tasks: Vec<String>,
    /// Optional cost impact (min, most likely, max) in currency units, sampled when the event fires.
    pub cost: Option<(f64, f64, f64)>,
}

/// Apply an uncertainty sidecar CSV to rows (for imported schedules, whose files carry no three-point data).
/// Key column: `activity_id`, `id` or `row` (activity ID, else 1-based row; see [`from_csv::resolve_ref`]) or `name`.
/// Value columns: `uncertainty` (class), `optimistic`, `pessimistic` (days). Returns the number of rows changed.
/// Rows whose key matches no task are not an error (a stale sidecar is common after edits in MS Project); they are
/// returned in [`SidecarCoverage::unmatched_rows`] together with the tasks the sidecar did not cover.
pub fn apply_uncertainty(rows: &mut [Row], text: &str) -> Result<SidecarCoverage, String> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).trim(csv::Trim::All).comment(Some(b'#')).from_reader(text.as_bytes());
    let hdr: Vec<String> = rdr.headers().map_err(|e| format!("uncertainty CSV header: {e}"))?.iter().map(|h| h.to_lowercase()).collect();
    let col = |names: &[&str]| names.iter().find_map(|n| hdr.iter().position(|h| h == n));
    let ck = col(&["activity_id", "id", "row"]);
    let cn = col(&["name"]);
    if ck.is_none() && cn.is_none() {
        return Err("uncertainty CSV needs an 'activity_id' (or id/row) or 'name' column".into());
    }
    let (cu, co, cp) = (col(&["uncertainty", "class"]), col(&["optimistic", "o"]), col(&["pessimistic", "p"]));
    let mut n = 0;
    let mut hit = vec![false; rows.len()];
    let mut unmatched = Vec::new();
    let key_col = ck.map(|c| hdr[c].as_str()).unwrap_or("");
    // Activity IDs match after trimming, case-insensitively; nothing looser matches silently.
    let mut plan_ids: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        if let Some(a) = r.activity_id.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            plan_ids.entry(a.to_uppercase()).or_default().push(i);
        }
    }
    let mut duplicate_plan_ids: Vec<String> = plan_ids.iter().filter(|(_, v)| v.len() > 1).map(|(k, _)| k.clone()).collect();
    duplicate_plan_ids.sort();
    let mut seen_line: Vec<Option<usize>> = vec![None; rows.len()];
    let mut dups = Vec::new();
    let mut near = Vec::new();
    for (k, rec) in rdr.records().enumerate() {
        let line = k + 2;
        let rec = rec.map_err(|e| format!("uncertainty line {line}: {e}"))?;
        let g = |c: Option<usize>| c.and_then(|c| rec.get(c)).unwrap_or("").trim().to_string();
        if rec.iter().all(|f| f.trim().is_empty()) {
            continue;
        }
        let i = match (g(ck), g(cn)) {
            (key, _) if !key.is_empty() => {
                let up = key.to_uppercase();
                let by_id = (key_col != "row").then(|| plan_ids.get(&up)).flatten();
                match by_id {
                    Some(v) if v.len() > 1 => {
                        return Err(format!(
                            "uncertainty line {line}: activity ID '{key}' is used by several tasks (rows {}); fix the plan or key by row",
                            v.iter().map(|i| (i + 1).to_string()).collect::<Vec<_>>().join(", ")
                        ));
                    }
                    Some(v) => v[0],
                    None => {
                        let row = key.strip_prefix('#').unwrap_or(if key_col == "activity_id" { "" } else { &key });
                        match row.parse::<usize>().ok().filter(|i| *i >= 1 && *i <= rows.len()) {
                            Some(r) => r - 1,
                            None => {
                                let c = near_misses_for(&key, &plan_ids);
                                if !c.is_empty() {
                                    near.push(NearMiss { key: key.clone(), candidates: c });
                                }
                                unmatched.push(key);
                                continue;
                            }
                        }
                    }
                }
            }
            (_, nm) if !nm.is_empty() => {
                let hits: Vec<usize> =
                    rows.iter().enumerate().filter(|(_, r)| r.name.trim().eq_ignore_ascii_case(&nm)).map(|(i, _)| i).collect();
                match hits.as_slice() {
                    [i] => *i,
                    [] => {
                        unmatched.push(nm);
                        continue;
                    }
                    _ => return Err(format!("uncertainty line {line}: several tasks named '{nm}'; use activity_id")),
                }
            }
            _ => continue,
        };
        if let Some(first) = seen_line[i] {
            dups.push(format!("lines {first} and {line} both set task {} '{}'", i + 1, rows[i].name));
            continue;
        }
        seen_line[i] = Some(line);
        let num = |c: Option<usize>, what: &str| -> Result<Option<f64>, String> {
            match g(c).as_str() {
                "" => Ok(None),
                v => v
                    .trim_end_matches(['d', 'D'])
                    .parse::<f64>()
                    .ok()
                    .filter(|x| x.is_finite() && *x >= 0.0)
                    .map(Some)
                    .ok_or(format!("uncertainty line {line}: bad {what} '{v}'")),
            }
        };
        let (o, p, u) = (num(co, "optimistic")?, num(cp, "pessimistic")?, g(cu));
        if let (Some(o), Some(p)) = (o, p)
            && o > p
        {
            return Err(format!("uncertainty line {line}: optimistic > pessimistic"));
        }
        let r = &mut rows[i];
        if !u.is_empty() {
            r.uncertainty = Some(u);
        }
        if o.is_some() {
            r.optimistic = o;
        }
        if p.is_some() {
            r.pessimistic = p;
        }
        hit[i] = true;
        n += 1;
    }
    if !dups.is_empty() {
        return Err(format!("duplicate sidecar keys: {}", dups.join("; ")));
    }
    // Tasks that get blanket --opt/--pess ranges: not covered by the sidecar, no own estimate or class, real work.
    let fallback = rows
        .iter()
        .enumerate()
        .filter(|(i, r)| !hit[*i] && r.dur > 0.0 && r.optimistic.is_none() && r.pessimistic.is_none() && r.uncertainty.is_none())
        .map(|(i, r)| CoverageTask { id: i + 1, activity_id: r.activity_id.clone(), name: r.name.clone() })
        .collect::<Vec<_>>();
    Ok(SidecarCoverage {
        rows_applied: n,
        tasks_covered: hit.iter().filter(|h| **h).count(),
        fallback_tasks: fallback,
        unmatched_rows: unmatched,
        near_misses: near,
        duplicate_plan_ids,
    })
}

/// Canonical form for near-miss detection: upper case, leading zeros of the numeric tail dropped (`A0050` → `A50`).
fn canon_id(s: &str) -> (String, Option<u64>) {
    let u = s.trim().to_uppercase();
    let p = u.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    (u[..p].to_string(), u[p..].parse().ok())
}

/// Plan activity IDs a stale or Excel-mangled key probably meant: same prefix and number (leading zeros lost), or a
/// numeric-only key whose number matches an ID's numeric tail (Excel stripped the prefix).
fn near_misses_for(key: &str, plan_ids: &HashMap<String, Vec<usize>>) -> Vec<String> {
    let (kp, kn) = canon_id(key);
    let Some(kn) = kn else { return vec![] };
    let mut v: Vec<String> = plan_ids
        .keys()
        .filter(|id| {
            let (p, n) = canon_id(id);
            n == Some(kn) && (p == kp || kp.is_empty())
        })
        .cloned()
        .collect();
    v.sort();
    v
}

/// A sidecar key that matched nothing but resembles plan activity IDs. Never applied automatically.
#[derive(Debug, Serialize, Clone)]
pub struct NearMiss {
    pub key: String,
    pub candidates: Vec<String>,
}

/// Coverage of an uncertainty sidecar (see [`apply_uncertainty`]).
#[derive(Debug, Serialize, Clone, Default)]
pub struct SidecarCoverage {
    /// Sidecar rows applied to a task.
    pub rows_applied: usize,
    /// Distinct tasks that took values from the sidecar.
    pub tasks_covered: usize,
    /// Tasks with duration that fell back to the blanket --opt/--pess range.
    pub fallback_tasks: Vec<CoverageTask>,
    /// Sidecar keys (activity ID, row or name) that matched no task. Complete list (the text report caps at 10).
    pub unmatched_rows: Vec<String>,
    /// Unmatched keys that look like a plan activity ID with lost leading zeros or prefix.
    pub near_misses: Vec<NearMiss>,
    /// Activity IDs used by more than one task in the plan (rows keyed by them are rejected).
    pub duplicate_plan_ids: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct CoverageTask {
    pub id: usize,
    pub activity_id: Option<String>,
    pub name: String,
}

impl SidecarCoverage {
    /// Coverage lines for the text report; the second line is a warning when the sidecar looks stale.
    pub fn render(&self) -> String {
        let mut s = format!(
            "Uncertainty sidecar: {} task(s) covered, {} fell back to --opt/--pess, {} sidecar row(s) matched no task\n",
            self.tasks_covered,
            self.fallback_tasks.len(),
            self.unmatched_rows.len()
        );
        if !self.fallback_tasks.is_empty() {
            let mut v: Vec<String> = self
                .fallback_tasks
                .iter()
                .take(10)
                .map(|t| format!("{} {}", t.activity_id.clone().unwrap_or_else(|| format!("#{}", t.id)), t.name))
                .collect();
            if self.fallback_tasks.len() > 10 {
                v.push(format!("… {} more", self.fallback_tasks.len() - 10));
            }
            s += &format!("  WARNING: sidecar does not cover {} task(s): {}\n", self.fallback_tasks.len(), v.join("; "));
        }
        if !self.unmatched_rows.is_empty() {
            let mut v: Vec<String> = self.unmatched_rows.iter().take(10).cloned().collect();
            if self.unmatched_rows.len() > 10 {
                v.push(format!("… {} more", self.unmatched_rows.len() - 10));
            }
            s += &format!("  WARNING: {} sidecar row(s) match no task (stale?): {}\n", self.unmatched_rows.len(), v.join(", "));
        }
        for m in &self.near_misses {
            s += &format!("  WARNING: sidecar key '{}' matches no task; did you mean {}? (not applied)\n", m.key, m.candidates.join(" / "));
        }
        if !self.duplicate_plan_ids.is_empty() {
            s += &format!("  WARNING: activity ID(s) used by several tasks in the plan: {}\n", self.duplicate_plan_ids.join(", "));
        }
        s
    }
    /// True when `--strict` should fail: a task fell back to --opt/--pess or a sidecar row matched nothing.
    pub fn is_stale(&self) -> bool {
        !self.fallback_tasks.is_empty() || !self.unmatched_rows.is_empty()
    }
}

/// Parse a RAID risk CSV: `id, name, probability, min_days, likely_days, max_days, tasks` (header required,
/// case-insensitive; `probability` as 0.3 or 30%; `tasks` = row numbers separated by `;`). Short forms `p`, `min`,
/// `likely`/`ml`, `max` are accepted.
pub fn parse_events(text: &str) -> Result<Vec<RiskEvent>, String> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).trim(csv::Trim::All).comment(Some(b'#')).from_reader(text.as_bytes());
    let hdr: Vec<String> = rdr.headers().map_err(|e| format!("risk CSV header: {e}"))?.iter().map(|h| h.to_lowercase()).collect();
    let col = |names: &[&str]| names.iter().find_map(|n| hdr.iter().position(|h| h == n));
    let ci = col(&["id"]);
    let cn = col(&["name", "risk", "description"]);
    let cp = col(&["probability", "p", "prob"]).ok_or("risk CSV needs a 'probability' column")?;
    let (cmin, cml, cmax) = (col(&["min_days", "min"]), col(&["likely_days", "likely", "ml", "most_likely"]), col(&["max_days", "max"]));
    let cmax = cmax.ok_or("risk CSV needs a 'max_days' column")?;
    let ct = col(&["tasks", "task_ids", "linked_tasks"]).ok_or("risk CSV needs a 'tasks' column")?;
    let (ccmin, ccml, ccmax) = (col(&["cost_min"]), col(&["cost_likely", "cost_ml"]), col(&["cost_max"]));
    let mut out = vec![];
    for (k, rec) in rdr.records().enumerate() {
        let line = k + 2;
        let rec = rec.map_err(|e| format!("risk line {line}: {e}"))?;
        let g = |c: Option<usize>| c.and_then(|c| rec.get(c)).unwrap_or("").trim().to_string();
        if rec.iter().all(|f| f.trim().is_empty()) {
            continue;
        }
        let num = |c: Option<usize>, what: &str| -> Result<Option<f64>, String> {
            match g(c).as_str() {
                "" => Ok(None),
                v => v
                    .trim_end_matches(['d', 'D'])
                    .parse::<f64>()
                    .ok()
                    .filter(|x| x.is_finite() && *x >= 0.0)
                    .map(Some)
                    .ok_or(format!("risk line {line}: bad {what} '{v}'")),
            }
        };
        let pv = g(Some(cp));
        let probability = match pv.strip_suffix('%') {
            Some(x) => x.trim().parse::<f64>().map(|x| x / 100.0),
            None => pv.parse::<f64>(),
        }
        .ok()
        .filter(|p| (0.0..=1.0).contains(p))
        .ok_or(format!("risk line {line}: bad probability '{pv}' (0-1 or 0-100%)"))?;
        let max_days = num(Some(cmax), "max_days")?.ok_or(format!("risk line {line}: missing max_days"))?;
        let likely_days = num(cml, "likely_days")?.unwrap_or(max_days);
        let min_days = num(cmin, "min_days")?.unwrap_or(likely_days.min(max_days));
        if !(min_days <= likely_days && likely_days <= max_days) {
            return Err(format!("risk line {line}: need min <= likely <= max"));
        }
        let tasks = g(Some(ct)).split([';', ',', ' ']).filter(|t| !t.is_empty()).map(|t| t.trim().to_string()).collect::<Vec<_>>();
        if tasks.is_empty() {
            return Err(format!("risk line {line}: no linked tasks"));
        }
        let cmx = num(ccmax, "cost_max")?;
        let cost = match cmx {
            None if g(ccmin).is_empty() && g(ccml).is_empty() => None,
            None => return Err(format!("risk line {line}: cost_min/cost_likely need cost_max")),
            Some(mx) => {
                let ml = num(ccml, "cost_likely")?.unwrap_or(mx);
                let mn = num(ccmin, "cost_min")?.unwrap_or(ml.min(mx));
                if !(mn <= ml && ml <= mx) {
                    return Err(format!("risk line {line}: need cost_min <= cost_likely <= cost_max"));
                }
                Some((mn, ml, mx))
            }
        };
        let id = Some(g(ci)).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("R{}", k + 1));
        out.push(RiskEvent {
            name: Some(g(cn)).filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone()),
            id,
            probability,
            min_days,
            likely_days,
            max_days,
            tasks,
            cost,
        });
    }
    Ok(out)
}

/// Parse `--class name=o,p` (factors of the most likely duration).
pub fn parse_class(s: &str) -> Result<(String, (f64, f64)), String> {
    let (n, v) = s.split_once('=').ok_or(format!("bad --class '{s}' (want name=0.9,1.6)"))?;
    let (o, p) = v.split_once(',').ok_or(format!("bad --class '{s}' (want name=0.9,1.6)"))?;
    let (o, p) = (
        o.trim().parse::<f64>().map_err(|_| format!("bad --class '{s}'"))?,
        p.trim().parse::<f64>().map_err(|_| format!("bad --class '{s}'"))?,
    );
    if !(0.0..=1.0).contains(&o) || p < 1.0 || !p.is_finite() {
        return Err(format!("--class '{s}': need 0 <= optimistic <= 1 <= pessimistic"));
    }
    Ok((n.trim().to_lowercase(), (o, p)))
}

#[derive(Debug, Serialize)]
pub struct Percentile {
    pub label: String,
    /// Working days (of `hours_per_day`) from the project start to the finish.
    pub working_days: f64,
    pub finish: NaiveDateTime,
}

#[derive(Debug, Serialize)]
pub struct TaskRisk {
    pub id: usize,
    pub name: String,
    /// Uncertainty class used (`custom` = CSV optimistic/pessimistic, `default` = --opt/--pess).
    pub class: String,
    pub optimistic_days: f64,
    pub most_likely_days: f64,
    pub pessimistic_days: f64,
    /// Share of runs (0–1) in which the task was on the driving path.
    pub criticality: f64,
    /// Pearson correlation of the task's sampled duration (incl. risk-event impacts) with the project finish.
    pub sensitivity: f64,
    /// Critical in the deterministic schedule.
    pub critical: bool,
}

#[derive(Debug, Serialize)]
pub struct EventRisk {
    pub id: String,
    pub name: String,
    pub probability: f64,
    /// Share of runs in which the event fired.
    pub hit_rate: f64,
    /// Mean project finish (working days) when the event fired minus when it did not.
    pub mean_finish_impact_days: Option<f64>,
    /// Expected cost over all runs (probability x mean sampled cost); only with cost columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_cost: Option<f64>,
}

/// Risk-cost distribution (sum of fired event costs per run). Duration-driven cost (overtime, extended general
/// conditions) is not modelled.
#[derive(Debug, Serialize)]
pub struct CostRisk {
    pub mean: f64,
    pub p50: f64,
    pub p80: f64,
    pub p90: f64,
    /// Share of runs finishing by the P80 date AND within the P80 risk cost (joint confidence; at most 80%).
    pub joint_p80: f64,
}

#[derive(Debug, Serialize)]
pub struct RiskReport {
    pub project_start: NaiveDate,
    pub status_date: Option<NaiveDate>,
    pub runs: usize,
    pub seed: u64,
    pub deterministic_working_days: f64,
    pub deterministic_finish: NaiveDateTime,
    /// Probability of finishing on or before the deterministic finish.
    pub p_deterministic: f64,
    pub mean_working_days: f64,
    pub percentiles: Vec<Percentile>,
    /// Leaf tasks, most critical first.
    pub tasks: Vec<TaskRisk>,
    /// Risk events, largest finish impact first (empty without a risk CSV).
    pub events: Vec<EventRisk>,
    /// Risk-event cost summary (only when at least one event has cost columns).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_cost: Option<CostRisk>,
    /// Uncertainty sidecar coverage (only with --uncertainty).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uncertainty_coverage: Option<SidecarCoverage>,
}

/// xoshiro256** seeded through splitmix64: small, fast, and reproducible across platforms and versions.
pub struct Rng([u64; 4]);

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        Rng([next(), next(), next(), next()])
    }
    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.0;
        let r = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        r
    }
    /// Uniform in (0, 1).
    pub fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
    pub fn normal(&mut self) -> f64 {
        let (u, v) = (self.uniform(), self.uniform());
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
    /// Gamma(shape ≥ 1, scale 1), Marsaglia–Tsang.
    pub fn gamma(&mut self, a: f64) -> f64 {
        if a < 1.0 {
            let u = self.uniform();
            return self.gamma(a + 1.0) * u.powf(1.0 / a);
        }
        let d = a - 1.0 / 3.0;
        let c = 1.0 / (9.0 * d).sqrt();
        loop {
            let x = self.normal();
            let v = (1.0 + c * x).powi(3);
            if v <= 0.0 {
                continue;
            }
            let u = self.uniform();
            if u.ln() < 0.5 * x * x + d - d * v + d * v.ln() {
                return d * v;
            }
        }
    }
    pub fn beta(&mut self, a: f64, b: f64) -> f64 {
        let x = self.gamma(a);
        let y = self.gamma(b);
        x / (x + y)
    }
}

/// PERT-beta sample with mode `m` on `[o, p]` (returns `m` when the range is empty).
pub fn pert(rng: &mut Rng, o: f64, m: f64, p: f64) -> f64 {
    if p <= o {
        return m;
    }
    let m = m.clamp(o, p);
    let a = 1.0 + 4.0 * (m - o) / (p - o);
    let b = 1.0 + 4.0 * (p - m) / (p - o);
    o + rng.beta(a, b) * (p - o)
}

/// Leaves on the driving path to the latest-finishing leaf.
fn driving_path(net: &Network, dur: &[f64], es: &[f64], ef: &[f64], out: &mut Vec<usize>) {
    out.clear();
    let Some(mut i) = net.topo.iter().copied().max_by(|&a, &b| ef[a].total_cmp(&ef[b]).then(b.cmp(&a))) else { return };
    for _ in 0..=net.n {
        out.push(i);
        let next = net.incoming[i]
            .iter()
            .map(|&k| &net.edges[k])
            .find(|(p, _, ty, lag)| (Network::link_value(ty, es[*p], ef[*p], *lag, dur[i]) - es[i]).abs() < 1e-6);
        match next {
            Some(e) => i = e.0,
            None => break,
        }
    }
}

/// Run the simulation over `rows` on calendar `cal`.
pub fn simulate(rows: &[Row], today: NaiveDate, cal: &CalendarSpec, opts: &RiskOptions) -> Result<RiskReport, String> {
    if opts.runs == 0 || opts.runs > 1_000_000 {
        return Err(format!("runs must be 1..=1000000 (got {})", opts.runs));
    }
    if !(opts.opt_factor >= 0.0 && opts.opt_factor <= 1.0 && opts.pess_factor >= 1.0 && opts.pess_factor.is_finite()) {
        return Err("need 0 <= --opt <= 1 <= --pess".into());
    }
    let n = rows.len();
    // Per-row (O, M, P) in days of remaining work, and the class used.
    let mut est = vec![(0.0, 0.0, 0.0); n];
    let mut class = vec![String::new(); n];
    for (i, r) in rows.iter().enumerate() {
        let m = r.remaining_days();
        let scale = if r.dur > 0.0 { m / r.dur } else { 1.0 };
        let (o, p, c) = match (r.optimistic, r.pessimistic, r.uncertainty.as_deref()) {
            (Some(_), _, _) | (_, Some(_), _) => {
                let (o, p) = (r.optimistic.unwrap_or(r.dur * opts.opt_factor), r.pessimistic.unwrap_or(r.dur * opts.pess_factor));
                if o > r.dur + 1e-9 || p + 1e-9 < r.dur {
                    return Err(format!(
                        "task {} '{}': need optimistic <= duration_days <= pessimistic ({o} / {} / {p})",
                        i + 1,
                        r.name,
                        r.dur
                    ));
                }
                (o * scale, p * scale, "custom".to_string())
            }
            (None, None, Some(c)) => {
                let (fo, fp) = opts.classes.get(c).copied().ok_or_else(|| {
                    let mut k: Vec<&String> = opts.classes.keys().collect();
                    k.sort();
                    format!(
                        "task {} '{}': unknown uncertainty class '{c}' (known: {})",
                        i + 1,
                        r.name,
                        k.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                    )
                })?;
                (m * fo, m * fp, c.to_string())
            }
            _ => (m * opts.opt_factor, m * opts.pess_factor, "default".to_string()),
        };
        est[i] = (o, m, p);
        class[i] = c;
    }
    let sopts = ScheduleOptions { status_date: opts.status_date, level: false };
    let (plan, _) = from_csv::schedule_full(rows, None, today, cal, &sopts)?;
    let mut net = Network::with_status(rows, today, cal, opts.status_date)?;
    let mpd = net.minutes_per_day;
    let leaf = |i: usize| !net.summary[i] && !rows[i].is_complete();
    // Risk event targets: leaves under each linked row.
    let mut targets: Vec<Vec<usize>> = vec![];
    for e in &opts.events {
        let mut t = vec![];
        for r in &e.tasks {
            let i = from_csv::resolve_ref(rows, r).map_err(|err| format!("risk {}: {err}", e.id))?;
            if net.summary[i] {
                t.extend((i + 1..n).take_while(|&k| rows[k].level > rows[i].level).filter(|&k| leaf(k)));
            } else if leaf(i) {
                t.push(i);
            }
        }
        t.sort();
        t.dedup();
        targets.push(t);
    }

    let det_dur: Vec<f64> = (0..n).map(|i| if leaf(i) { est[i].1 * mpd } else { 0.0 }).collect();
    let (_, def) = net.forward(&det_dur);
    let det = net.topo.iter().map(|&i| def[i]).fold(0.0, f64::max);

    let mut rng = Rng::new(opts.seed);
    let mut finishes = Vec::with_capacity(opts.runs);
    let mut crit = vec![0usize; n];
    let mut dur = vec![0.0; n];
    let mut path = vec![];
    let (mut sx, mut sxx, mut sxy) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    let (mut sy, mut syy) = (0.0, 0.0);
    let ne = opts.events.len();
    let (mut hits, mut fin_hit) = (vec![0usize; ne], vec![0.0; ne]);
    let has_cost = opts.events.iter().any(|e| e.cost.is_some());
    let mut ev_cost = vec![0.0; ne];
    let mut costs: Vec<f64> = Vec::with_capacity(if has_cost { opts.runs } else { 0 });
    for _ in 0..opts.runs {
        for i in 0..n {
            dur[i] = if leaf(i) { pert(&mut rng, est[i].0, est[i].1, est[i].2) * mpd } else { 0.0 };
        }
        let mut fired = vec![false; ne];
        let mut run_cost = 0.0;
        for (k, e) in opts.events.iter().enumerate() {
            if rng.uniform() < e.probability {
                fired[k] = true;
                let imp = pert(&mut rng, e.min_days, e.likely_days, e.max_days) * mpd;
                for &i in &targets[k] {
                    dur[i] += imp;
                }
                if let Some((a, b, c)) = e.cost {
                    let x = pert(&mut rng, a, b, c);
                    run_cost += x;
                    ev_cost[k] += x;
                }
            }
        }
        let (es, ef) = net.forward(&dur);
        let f = net.topo.iter().map(|&i| ef[i]).fold(0.0, f64::max);
        finishes.push(f);
        if has_cost {
            costs.push(run_cost);
        }
        let fd = f / mpd;
        sy += fd;
        syy += fd * fd;
        for i in 0..n {
            if leaf(i) {
                let x = dur[i] / mpd;
                sx[i] += x;
                sxx[i] += x * x;
                sxy[i] += x * fd;
            }
        }
        for k in 0..ne {
            if fired[k] {
                hits[k] += 1;
                fin_hit[k] += fd;
            }
        }
        driving_path(&net, &dur, &es, &ef, &mut path);
        for &i in &path {
            crit[i] += 1;
        }
    }
    let runs = opts.runs as f64;
    let total_fin: f64 = sy;
    let corr = |i: usize| {
        let cov = sxy[i] / runs - sx[i] / runs * sy / runs;
        let vx = sxx[i] / runs - (sx[i] / runs).powi(2);
        let vy = syy / runs - (sy / runs).powi(2);
        if vx > 1e-12 && vy > 1e-12 { (cov / (vx * vy).sqrt()).clamp(-1.0, 1.0) } else { 0.0 }
    };
    let mut events: Vec<EventRisk> = opts
        .events
        .iter()
        .enumerate()
        .map(|(k, e)| {
            let h = hits[k] as f64;
            let impact = (hits[k] > 0 && hits[k] < opts.runs).then(|| fin_hit[k] / h - (total_fin - fin_hit[k]) / (runs - h));
            EventRisk {
                id: e.id.clone(),
                name: e.name.clone(),
                probability: e.probability,
                hit_rate: h / runs,
                mean_finish_impact_days: impact,
                expected_cost: e.cost.map(|_| ev_cost[k] / runs),
            }
        })
        .collect();
    events.sort_by(|a, b| b.mean_finish_impact_days.unwrap_or(0.0).total_cmp(&a.mean_finish_impact_days.unwrap_or(0.0)));
    let event_cost = has_cost.then(|| {
        let mut sc = costs.clone();
        sc.sort_by(f64::total_cmp);
        let mut sf = finishes.clone();
        sf.sort_by(f64::total_cmp);
        let qq = |v: &[f64], x: f64| v[((x * runs) as usize).min(v.len() - 1)];
        let (f80, c80) = (qq(&sf, 0.8), qq(&sc, 0.8));
        let joint = finishes.iter().zip(&costs).filter(|(f, c)| **f <= f80 + 1e-9 && **c <= c80 + 1e-9).count() as f64 / runs;
        CostRisk { mean: costs.iter().sum::<f64>() / runs, p50: qq(&sc, 0.5), p80: c80, p90: qq(&sc, 0.9), joint_p80: joint }
    });
    finishes.sort_by(f64::total_cmp);
    let q = |x: f64| finishes[((x * runs) as usize).min(finishes.len() - 1)];
    let tl = &mut net.timeline;
    let percentiles = [("P10", 0.1), ("P50", 0.5), ("P80", 0.8), ("P90", 0.9)]
        .iter()
        .map(|&(l, x)| Percentile { label: l.into(), working_days: q(x) / mpd, finish: tl.to_dt(q(x), true) })
        .collect();
    let mut tasks: Vec<TaskRisk> = (0..n)
        .filter(|&i| leaf(i))
        .map(|i| TaskRisk {
            id: i + 1,
            name: rows[i].name.clone(),
            class: class[i].clone(),
            optimistic_days: est[i].0,
            most_likely_days: est[i].1,
            pessimistic_days: est[i].2,
            criticality: crit[i] as f64 / runs,
            sensitivity: corr(i),
            critical: plan.tasks[i].critical,
        })
        .collect();
    tasks.sort_by(|a, b| b.criticality.total_cmp(&a.criticality).then(b.sensitivity.total_cmp(&a.sensitivity)).then(a.id.cmp(&b.id)));
    Ok(RiskReport {
        project_start: tl.base(),
        status_date: opts.status_date,
        runs: opts.runs,
        seed: opts.seed,
        deterministic_working_days: det / mpd,
        deterministic_finish: tl.to_dt(det, true),
        p_deterministic: finishes.iter().filter(|f| **f <= det + 1e-6).count() as f64 / runs,
        mean_working_days: sy / runs,
        percentiles,
        tasks,
        events,
        event_cost,
        uncertainty_coverage: None,
    })
}

/// Plain-text report.
pub fn render_text(r: &RiskReport) -> String {
    use std::fmt::Write;
    let dt = |d: &NaiveDateTime| d.format("%Y-%m-%d %H:%M").to_string();
    let mut s = String::new();
    let _ = writeln!(
        s,
        "Project start {}; deterministic finish {:.1} wd -> {}",
        r.project_start,
        r.deterministic_working_days,
        dt(&r.deterministic_finish)
    );
    if let Some(c) = &r.uncertainty_coverage {
        s += &c.render();
    }
    if let Some(sd) = r.status_date {
        let _ = writeln!(s, "Status date {sd} (working days count from the project start)");
    }
    let _ = writeln!(s, "P(finish <= deterministic) = {:.0}%  ({} runs, seed {})", r.p_deterministic * 100.0, r.runs, r.seed);
    let _ = writeln!(s, "Mean: {:.1} wd", r.mean_working_days);
    for p in &r.percentiles {
        let _ = writeln!(s, "{}: {:.1} wd -> {}", p.label, p.working_days, dt(&p.finish));
    }
    let _ = writeln!(s, "Criticality index and sensitivity (share of runs on the driving path; duration-finish correlation):");
    for t in r.tasks.iter().filter(|t| t.criticality > 0.0 || t.sensitivity.abs() >= 0.1) {
        let flag = if t.critical { " *" } else { "" };
        let name: String = t.name.chars().take(40).collect();
        let _ = writeln!(s, "  {:>4} {:<40} {:6.1}%  r={:+.2}  {}{flag}", t.id, name, t.criticality * 100.0, t.sensitivity, t.class);
    }
    let _ = writeln!(s, "  (* = critical in the deterministic schedule)");
    let mut sens: Vec<&TaskRisk> = r.tasks.iter().filter(|t| t.sensitivity >= 0.005).collect();
    sens.sort_by(|a, b| b.sensitivity.total_cmp(&a.sensitivity));
    if !sens.is_empty() {
        let _ = writeln!(s, "Top drivers by sensitivity (tornado):");
        for t in sens.iter().take(10) {
            let bar = "#".repeat((t.sensitivity * 30.0).round() as usize);
            let name: String = t.name.chars().take(40).collect();
            let _ = writeln!(s, "  {:>4} {:<40} {:+.2} {bar}", t.id, name, t.sensitivity);
        }
    }
    if !r.events.is_empty() {
        let _ = writeln!(s, "Risk events (probability, hit rate, mean finish impact when it fires):");
        for e in &r.events {
            let imp = e.mean_finish_impact_days.map(|v| format!("{v:+.1} wd")).unwrap_or_else(|| "n/a".into());
            let name: String = e.name.chars().take(40).collect();
            let cost = e.expected_cost.map(|c| format!("  E[cost]={c:.0}")).unwrap_or_default();
            let _ = writeln!(s, "  {:<8} {:<40} p={:4.0}% hit={:4.0}% {imp}{cost}", e.id, name, e.probability * 100.0, e.hit_rate * 100.0);
        }
    }
    if let Some(c) = &r.event_cost {
        let _ = writeln!(
            s,
            "Risk-event cost: mean {:.0}, P50 {:.0}, P80 {:.0}, P90 {:.0}; joint P80 (date and cost) {:.0}%",
            c.mean,
            c.p50,
            c.p80,
            c.p90,
            c.joint_p80 * 100.0
        );
        let _ = writeln!(s, "  (event costs only; overtime and extended general conditions are not modelled)");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::from_csv::parse_rows;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn rng_is_reproducible_and_beta_has_right_mean() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        assert_eq!(a.next_u64(), b.next_u64());
        let mut r = Rng::new(1);
        let n = 20_000;
        let m: f64 = (0..n).map(|_| r.beta(2.0, 5.0)).sum::<f64>() / n as f64;
        assert!((m - 2.0 / 7.0).abs() < 0.01, "beta(2,5) mean {m}");
        // PERT mean = (O + 4M + P) / 6.
        let pm: f64 = (0..n).map(|_| pert(&mut r, 8.0, 10.0, 15.0)).sum::<f64>() / n as f64;
        assert!((pm - (8.0 + 40.0 + 15.0) / 6.0).abs() < 0.05, "pert mean {pm}");
        assert_eq!(pert(&mut r, 3.0, 3.0, 3.0), 3.0);
    }

    const CSV: &str = "name,start,duration_days,predecessors,optimistic,pessimistic\n\
        A,2026-11-02,10,,,\n\
        B,,5,1,5,5\n\
        C,,4,1,2,12\n\
        Done,,0,2;3,,\n";

    #[test]
    fn percentiles_and_criticality() {
        let rows = parse_rows(CSV).unwrap();
        let opts = RiskOptions { runs: 4000, ..Default::default() };
        let r = simulate(&rows, d("2026-01-01"), &CalendarSpec::default(), &opts).unwrap();
        assert_eq!(r.deterministic_working_days, 15.0);
        assert_eq!(r.deterministic_finish.to_string(), "2026-11-20 17:00:00");
        let w: Vec<f64> = r.percentiles.iter().map(|p| p.working_days).collect();
        assert!(w.windows(2).all(|x| x[0] <= x[1]));
        assert!(w[0] >= 8.0 * 1.0 + 5.0 - 1e-9 && w[3] <= 15.0 + 12.0);
        let get = |id| r.tasks.iter().find(|t| t.id == id).unwrap();
        assert_eq!(get(1).criticality, 1.0);
        // C (4d, up to 12d) beats B's fixed 5d only part of the time.
        assert!(get(3).criticality > 0.05 && get(3).criticality < 0.95, "{}", get(3).criticality);
        assert!((get(2).criticality + get(3).criticality - 1.0).abs() < 0.05);
        // Same seed, same answer.
        let r2 = simulate(&rows, d("2026-01-01"), &CalendarSpec::default(), &opts).unwrap();
        assert_eq!(r2.percentiles[1].working_days, r.percentiles[1].working_days);
        assert!(render_text(&r).contains("P80:"));
    }

    #[test]
    fn honours_holidays() {
        let rows = parse_rows("name,start,duration_days\nA,2026-11-23,5\n").unwrap();
        let cal = CalendarSpec { holidays: "US".into(), ..Default::default() };
        let opts = RiskOptions { runs: 10, opt_factor: 1.0, pess_factor: 1.0, ..Default::default() };
        let r = simulate(&rows, d("2026-01-01"), &cal, &opts).unwrap();
        // Thanksgiving (Thu 26th) pushes a 5-day task from Mon 23rd to Mon 30th.
        assert_eq!(r.percentiles[2].finish.to_string(), "2026-11-30 17:00:00");
        assert!(simulate(&rows, d("2026-01-01"), &cal, &RiskOptions { runs: 0, ..Default::default() }).is_err());
        let bad = parse_rows("name,duration_days,optimistic\nA,5,7\n").unwrap();
        assert!(simulate(&bad, d("2026-01-01"), &cal, &opts).is_err());
    }
}

/// Convert a parsed schedule (any format mpptool reads) into CSV-style rows plus its project calendar, so CPM and
/// Monte Carlo run on its task network. Progress is carried over (actual start/finish, % complete → remaining
/// duration) and the status date is returned (the file's, or the latest actual date); a task's start is anchored with its SNET/MSO constraint date, or its actual/current start when it has no
/// predecessors. The project summary (UID 0) and blank rows are skipped.
pub fn rows_from_plan(plan: &crate::model::Plan) -> Result<(Vec<Row>, CalendarSpec, Option<NaiveDate>), String> {
    let hpd = if plan.minutes_per_day > 0 { plan.minutes_per_day as f64 / 60.0 } else { 8.0 };
    let tasks: Vec<&crate::model::TaskRec> =
        plan.tasks.iter().filter(|t| t.uid != 0 && !(t.name.trim().is_empty() && t.start.is_none() && t.finish.is_none())).collect();
    if tasks.is_empty() {
        return Err("schedule has no tasks".into());
    }
    let row_of: std::collections::HashMap<i32, usize> = tasks.iter().enumerate().map(|(i, t)| (t.uid, i + 1)).collect();
    let min_level = tasks.iter().map(|t| t.outline_level).min().unwrap_or(1);
    let mut rows = vec![];
    for t in &tasks {
        let preds: Vec<(usize, String, f64)> = t
            .predecessors
            .iter()
            .filter_map(|p| row_of.get(&p.predecessor_uid).map(|&r| (r, p.link_type.to_uppercase(), p.lag_hours / hpd)))
            .filter(|p| ["FS", "SS", "FF", "SF"].contains(&p.1.as_str()))
            .collect();
        let dated = matches!(t.constraint_type, Some(2) | Some(4)).then_some(t.constraint_date).flatten();
        let pct = t.percent_complete.clamp(0.0, 100.0);
        let done = !t.summary && (pct >= 100.0 || t.actual_finish.is_some());
        let started = !t.summary && (done || pct > 0.0 || t.actual_start.is_some());
        let actual_start = started.then(|| t.actual_start.or(t.start)).flatten().map(|d| d.date());
        let actual_finish = if done { t.actual_finish.or(t.finish).map(|d| d.date()) } else { None };
        let start = dated.or(if t.predecessors.is_empty() { t.start } else { None }).map(|d| d.date());
        let res: Vec<String> = plan
            .assignments
            .iter()
            .filter(|a| a.task_uid == t.uid)
            .filter_map(|a| plan.resources.iter().find(|r| r.uid == a.resource_uid && r.rtype.eq_ignore_ascii_case("work")))
            .map(|r| r.name.clone())
            .collect();
        rows.push(Row {
            name: if t.name.trim().is_empty() { format!("(UID {})", t.uid) } else { t.name.clone() },
            start,
            dur: if t.summary { 0.0 } else { t.duration_hours.max(0.0) / hpd },
            preds,
            level: (t.outline_level - min_level + 1).max(1),
            activity_id: t.activity_id.clone(),
            res,
            actual_start: actual_start.filter(|_| actual_finish.is_some() || started),
            actual_finish: actual_finish.filter(|_| actual_start.is_some()),
            percent_complete: started.then_some(pct),
            ..Default::default()
        });
    }
    let spec = crate::calendar::spec_from_plan(plan);
    // Status date: the file's, else the latest actual date (so actuals are never "in the future").
    let status = plan.status_date.map(|d| d.date()).or_else(|| rows.iter().flat_map(|r| [r.actual_start, r.actual_finish]).flatten().max());
    let status = status.map(|sd| rows.iter().flat_map(|r| [r.actual_start, r.actual_finish]).flatten().fold(sd, |a, b| a.max(b)));
    Ok((rows, spec, status))
}

/// Read a simple JSON task list (the `tasks --json` output of mpptool, or the same shape written by hand) into a plan:
/// an array of objects with `uid`, `name`, `outline_level`, `duration_hours`, optional `start`, `summary`,
/// `percent_complete`, `constraint_type`, `constraint_date` and `predecessors` [{`predecessor_uid`, `link_type`, `lag_hours`}].
pub fn plan_from_json(text: &str) -> Result<crate::model::Plan, String> {
    use crate::model::{Plan, Pred, TaskRec};
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("JSON: {e}"))?;
    let arr = v
        .as_array()
        .or_else(|| v.get("tasks").and_then(|t| t.as_array()))
        .ok_or("JSON must be an array of tasks (or {\"tasks\": [...]})")?;
    let dt = |x: &serde_json::Value| {
        x.as_str().and_then(|s| {
            NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
                .ok()
                .or_else(|| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(8, 0, 0)))
        })
    };
    let mut plan = Plan { format: "JSON".into(), minutes_per_day: 480, minutes_per_week: 2400, ..Default::default() };
    for (i, t) in arr.iter().enumerate() {
        let f = |k: &str| t.get(k).and_then(|x| x.as_f64());
        let uid = t.get("uid").and_then(|x| x.as_i64()).unwrap_or(i as i64 + 1) as i32;
        let preds = t
            .get("predecessors")
            .and_then(|p| p.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        Some(Pred {
                            predecessor_uid: p.get("predecessor_uid")?.as_i64()? as i32,
                            predecessor_id: None,
                            link_type: p.get("link_type").and_then(|x| x.as_str()).unwrap_or("FS").to_string(),
                            lag_hours: p.get("lag_hours").and_then(|x| x.as_f64()).unwrap_or(0.0),
                            lag_display: String::new(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        plan.tasks.push(TaskRec {
            uid,
            id: Some(i as i32 + 1),
            name: t.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            outline_level: t.get("outline_level").and_then(|x| x.as_i64()).unwrap_or(1) as i32,
            start: t.get("start").and_then(dt),
            duration_hours: f("duration_hours").ok_or(format!("JSON task {}: missing duration_hours", i + 1))?,
            percent_complete: f("percent_complete").unwrap_or(0.0),
            summary: t.get("summary").and_then(|x| x.as_bool()).unwrap_or(false),
            constraint_type: t.get("constraint_type").and_then(|x| x.as_i64()).map(|c| c as i32),
            constraint_date: t.get("constraint_date").and_then(dt),
            predecessors: preds,
            ..Default::default()
        });
    }
    Ok(plan)
}

/// Load rows + calendar from any input: `.csv` (from-csv format), `.json` (task list) or any schedule mpptool reads.
/// `csv_cal` applies to CSV input; imported schedules use their own project calendar.
pub fn load_any(
    path: &std::path::Path,
    csv_cal: &CalendarSpec,
    ropts: &crate::ReadOptions,
    engine: crate::Engine,
) -> Result<(Vec<Row>, CalendarSpec, Option<NaiveDate>), String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let name = |e: String| format!("{}: {e}", path.display());
    match ext.as_str() {
        "csv" => {
            let text = std::fs::read_to_string(path).map_err(|e| name(e.to_string()))?;
            Ok((from_csv::parse_rows(&text).map_err(name)?, csv_cal.clone(), None))
        }
        "json" => {
            let text = std::fs::read_to_string(path).map_err(|e| name(e.to_string()))?;
            rows_from_plan(&plan_from_json(&text).map_err(name)?).map_err(name)
        }
        _ => rows_from_plan(&crate::read(path, ropts, engine)?).map_err(name),
    }
}

/// One CPM row for `cpm` output.
#[derive(Debug, Serialize)]
pub struct CpmRow {
    pub row: usize,
    pub name: String,
    pub outline_level: i32,
    pub summary: bool,
    pub early_start: Option<NaiveDateTime>,
    pub early_finish: Option<NaiveDateTime>,
    pub remaining_days: f64,
    pub total_float_days: f64,
    pub free_float_days: f64,
    pub critical: bool,
}

/// Critical path (forward/backward pass, total and free float) of rows on `cal`, in working days of `hours_per_day`.
pub fn cpm(rows: &[Row], today: NaiveDate, cal: &CalendarSpec, opts: &ScheduleOptions) -> Result<Vec<CpmRow>, String> {
    let (p, _) = from_csv::schedule_full(rows, None, today, cal, opts)?;
    let hpd = cal.hours_per_day;
    Ok(p.tasks
        .iter()
        .enumerate()
        .map(|(i, t)| CpmRow {
            remaining_days: if t.summary { t.duration_hours / hpd } else { rows[i].remaining_days() },
            row: i + 1,
            name: t.name.clone(),
            outline_level: t.outline_level,
            summary: t.summary,
            early_start: t.start,
            early_finish: t.finish,
            total_float_days: t.total_slack_hours.unwrap_or(0.0) / hpd,
            free_float_days: t.free_slack_hours.unwrap_or(0.0) / hpd,
            critical: t.critical,
        })
        .collect())
}

#[cfg(test)]
mod event_tests {
    use super::*;
    use crate::from_csv::parse_rows;
    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn risk_events_classes_and_sensitivity() {
        let rows = parse_rows(
            "name,start,duration_days,predecessors,uncertainty\nA,2026-11-02,10,,fixed\nB,,5,1,vendor\nC,,5,1,fixed\nDone,,0,2;3,\n",
        )
        .unwrap();
        let ev = parse_events("id,name,probability,min_days,likely_days,max_days,tasks\nR1,Crane down,100%,3,3,3,3\n").unwrap();
        let opts = RiskOptions { runs: 2000, events: ev, ..Default::default() };
        let r = simulate(&rows, d("2026-01-01"), &CalendarSpec::default(), &opts).unwrap();
        // A and C are fixed; the certain 3-day event makes C 8 days, B is vendor (5..10 days).
        let c = r.tasks.iter().find(|t| t.id == 3).unwrap();
        assert_eq!(c.class, "fixed");
        assert!(r.percentiles[0].working_days >= 18.0 - 1e-9, "{:?}", r.percentiles[0]);
        assert_eq!(r.events[0].hit_rate, 1.0);
        assert!(r.events[0].mean_finish_impact_days.is_none(), "always fires: no comparison");
        let b = r.tasks.iter().find(|t| t.id == 2).unwrap();
        assert!(b.sensitivity > 0.3, "vendor task drives the finish when it exceeds 8 days: {}", b.sensitivity);
        assert_eq!(c.sensitivity, 0.0, "a fixed task has no variance");
        let bad = parse_rows("name,duration_days,uncertainty\nA,5,magic\n").unwrap();
        assert!(simulate(&bad, d("2026-01-01"), &CalendarSpec::default(), &opts).unwrap_err().contains("unknown uncertainty class"));
        assert!(parse_events("id,probability,max_days,tasks\nR,1.5,3,1\n").is_err());
        assert!(parse_events("id,probability,min_days,likely_days,max_days,tasks\nR,0.2,5,3,4,1\n").is_err());
        assert_eq!(parse_class("vendor=0.9,2.5").unwrap(), ("vendor".into(), (0.9, 2.5)));
        assert!(parse_class("x=1.2,0.5").is_err());
    }

    #[test]
    fn common_cause_event_hits_all_linked_tasks() {
        // Two parallel 10-day paths; one event delays both by 5 days together.
        let rows = parse_rows("name,start,duration_days,predecessors\nA,2026-11-02,10,\nB,2026-11-02,10,\nDone,,0,1;2\n").unwrap();
        let ev = parse_events("id,probability,min_days,likely_days,max_days,tasks\nR1,0.5,5,5,5,1;2\n").unwrap();
        let opts = RiskOptions { runs: 4000, opt_factor: 1.0, pess_factor: 1.0, events: ev, ..Default::default() };
        let r = simulate(&rows, d("2026-01-01"), &CalendarSpec::default(), &opts).unwrap();
        let imp = r.events[0].mean_finish_impact_days.unwrap();
        assert!((imp - 5.0).abs() < 1e-6, "{imp}");
        assert_eq!(r.percentiles[3].working_days, 15.0);
        assert_eq!(r.percentiles[0].working_days, 10.0);
    }

    #[test]
    fn risk_event_cost_and_activity_ids() {
        let csv = "activity_id,name,duration,predecessors\nA10,Dig,5,\nA20,Pour,5,A10\nA30,Cure,3,A20FS+1\n";
        let rows = crate::from_csv::parse_rows(csv).unwrap();
        assert_eq!(rows[2].preds[0].0, 2);
        assert_eq!(rows[2].preds[0].2, 1.0);
        let ev = parse_events("id,probability,max_days,tasks,cost_min,cost_likely,cost_max\nR1,100%,2,A20,1000,1000,1000\n").unwrap();
        let opts = RiskOptions { runs: 500, events: ev, ..Default::default() };
        let r = simulate(&rows, NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(), &CalendarSpec::default(), &opts).unwrap();
        let c = r.event_cost.unwrap();
        assert!((c.mean - 1000.0).abs() < 1e-6 && (c.p80 - 1000.0).abs() < 1e-6);
        assert!((r.events[0].expected_cost.unwrap() - 1000.0).abs() < 1e-6);
        let bad = parse_events("id,probability,max_days,tasks\nR1,1,2,ZZZ\n").unwrap();
        assert!(
            simulate(
                &rows,
                NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
                &CalendarSpec::default(),
                &RiskOptions { events: bad, ..Default::default() }
            )
            .is_err()
        );
    }

    #[test]
    fn sidecar_keys_normalize_and_report_near_misses() {
        let plan = "activity_id,name,duration_days\nA0050,Dig,5\nA0060,Pour,5\nB70,Cure,3\nX1,Spare,2\nX2,Spare 2,2\n";
        let mut rows = parse_rows(plan).unwrap();
        rows[4].activity_id = Some("x1".into()); // imported plans can carry duplicate IDs (case-insensitively)
        // Trim + case-insensitive IDs and names match; lost leading zeros / prefix do not, but are reported.
        let cov = apply_uncertainty(&mut rows, "activity_id,name,uncertainty\n  a0050 ,,vendor\n,  pour ,labor\nA50,,fixed\n70,,fixed\n")
            .unwrap();
        assert_eq!(rows[0].uncertainty.as_deref(), Some("vendor"));
        assert_eq!(rows[1].uncertainty.as_deref(), Some("labor"));
        assert_eq!(rows[2].uncertainty, None, "numeric-only key is not a row number under an activity_id header");
        assert_eq!(cov.unmatched_rows, vec!["A50", "70"]);
        assert_eq!(cov.near_misses.len(), 2);
        assert_eq!(cov.near_misses[0].candidates, vec!["A0050"]);
        assert_eq!(cov.near_misses[1].candidates, vec!["B70"]);
        assert_eq!(cov.duplicate_plan_ids, vec!["X1"]);
        assert!(cov.render().contains("did you mean A0050?"));
        // Duplicate sidecar keys (after normalization) are an error listing them.
        let e = apply_uncertainty(&mut rows, "activity_id,uncertainty\nA0050,vendor\na0050 ,fixed\nB70,labor\nb70,labor\n").unwrap_err();
        assert!(e.contains("lines 2 and 3") && e.contains("lines 4 and 5"), "{e}");
        // A sidecar row keyed by a duplicated plan activity ID is ambiguous.
        assert!(apply_uncertainty(&mut rows, "activity_id,uncertainty\nX1,vendor\n").unwrap_err().contains("several tasks"));
    }
}
