//! Portfolio / backlog prioritization by Weighted Shortest Job First (WSJF).
//!
//! SAFe form: WSJF = (user-business value + time criticality + risk reduction/opportunity enablement) ÷ job size, all
//! relative scores (modified Fibonacci 1, 2, 3, 5, 8, 13, 20, 40, 100). Reinertsen's original form (CD3) divides an
//! economic cost of delay per week by duration in weeks; with those columns the report also compares the total cost of
//! delay of the WSJF sequence against the input order, and with `cost` and a budget it marks the funding cut line.
use crate::lps::{Table, get};
use serde::Serialize;

const FIB: [f64; 10] = [0.0, 1.0, 2.0, 3.0, 5.0, 8.0, 13.0, 20.0, 40.0, 100.0];

#[derive(Debug, Serialize, Clone)]
pub struct Item {
    pub rank: usize,
    pub id: String,
    pub name: String,
    pub business_value: Option<f64>,
    pub time_criticality: Option<f64>,
    pub risk_reduction: Option<f64>,
    pub job_size: Option<f64>,
    /// Relative cost of delay (BV + TC + RR/OE).
    pub cost_of_delay: Option<f64>,
    pub wsjf: Option<f64>,
    /// Economic cost of delay per week and duration (weeks), and CD3 = CoD ÷ duration.
    pub cod_per_week: Option<f64>,
    pub duration_weeks: Option<f64>,
    pub cd3: Option<f64>,
    pub cost: Option<f64>,
    pub cumulative_cost: Option<f64>,
    /// Within the budget when the items are funded in rank order.
    pub funded: Option<bool>,
    /// Input row order (1-based).
    pub input_order: usize,
}

#[derive(Debug, Serialize)]
pub struct Ranking {
    /// `wsjf` (relative scores) or `cd3` (economic cost of delay ÷ duration).
    pub method: String,
    pub items: Vec<Item>,
    /// With cod_per_week and duration_weeks, delivering items one after another: total cost of delay of the ranked
    /// order and of the input order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_cost_ranked: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_cost_input_order: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<f64>,
    /// Scores that are not on the modified Fibonacci scale (a common sign of absolute rather than relative scoring).
    pub warnings: Vec<String>,
}

/// CSV columns: `id`, `name`, `business_value` (or `user_business_value`/`value`/`ubv`), `time_criticality` (`tc`),
/// `risk_reduction` (`rr_oe`/`opportunity_enablement`), `job_size` (`size`); optional `cod_per_week`,
/// `duration_weeks`, `cost`. Rank by CD3 when every item has cod_per_week and duration_weeks and `by_cd3` is set, or
/// when the relative columns are absent.
pub fn rank(text: &str, budget: Option<f64>, by_cd3: bool) -> Result<Ranking, String> {
    let t = Table::parse(text, "wsjf")?;
    let ci = t.col(&["id", "key"]);
    let cn = t.col(&["name", "title", "feature", "epic", "project"]);
    let cbv = t.col(&["business_value", "user_business_value", "value", "ubv", "bv"]);
    let ctc = t.col(&["time_criticality", "tc"]);
    let crr = t.col(&["risk_reduction", "rr_oe", "rr", "opportunity_enablement", "risk_reduction_opportunity_enablement"]);
    let cjs = t.col(&["job_size", "size", "effort"]);
    let (ccod, cdur, ccost) =
        (t.col(&["cod_per_week", "cost_of_delay_per_week", "cod"]), t.col(&["duration_weeks", "duration"]), t.col(&["cost", "budget"]));
    let relative = cbv.is_some() || ctc.is_some() || crr.is_some();
    if relative && cjs.is_none() {
        return Err("wsjf CSV needs a 'job_size' column".into());
    }
    if !relative && (ccod.is_none() || cdur.is_none()) {
        return Err("wsjf CSV needs business_value/time_criticality/risk_reduction + job_size, or cod_per_week + duration_weeks".into());
    }
    let mut warnings = vec![];
    let mut items = vec![];
    for (k, (line, r)) in t.recs.iter().enumerate() {
        let num = |c: Option<usize>, what: &str| -> Result<Option<f64>, String> {
            match get(r, c).trim_start_matches('$').replace(',', "").as_str() {
                "" => Ok(None),
                v => v
                    .parse::<f64>()
                    .ok()
                    .filter(|x| x.is_finite() && *x >= 0.0)
                    .map(Some)
                    .ok_or(format!("wsjf line {line}: bad {what} '{v}'")),
            }
        };
        let (bv, tc, rr, js) =
            (num(cbv, "business_value")?, num(ctc, "time_criticality")?, num(crr, "risk_reduction")?, num(cjs, "job_size")?);
        let (cod, dur, cost) = (num(ccod, "cod_per_week")?, num(cdur, "duration_weeks")?, num(ccost, "cost")?);
        let id = Some(get(r, ci).to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("#{}", k + 1));
        let codr = relative.then(|| bv.unwrap_or(0.0) + tc.unwrap_or(0.0) + rr.unwrap_or(0.0));
        if relative {
            match js {
                None => return Err(format!("wsjf line {line}: missing job_size")),
                Some(j) if j <= 0.0 => return Err(format!("wsjf line {line}: job_size must be > 0")),
                _ => {}
            }
            for (n, v) in [("business_value", bv), ("time_criticality", tc), ("risk_reduction", rr), ("job_size", js)] {
                if let Some(v) = v
                    && !FIB.iter().any(|f| (f - v).abs() < 1e-9)
                {
                    warnings.push(format!("{id}: {n} {v} is not on the modified Fibonacci scale"));
                }
            }
        }
        if dur.is_some_and(|d| d <= 0.0) {
            return Err(format!("wsjf line {line}: duration_weeks must be > 0"));
        }
        items.push(Item {
            rank: 0,
            name: Some(get(r, cn).to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone()),
            id,
            business_value: bv,
            time_criticality: tc,
            risk_reduction: rr,
            job_size: js,
            cost_of_delay: codr,
            wsjf: codr.zip(js).map(|(c, j)| c / j),
            cod_per_week: cod,
            duration_weeks: dur,
            cd3: cod.zip(dur).map(|(c, d)| c / d),
            cost,
            cumulative_cost: None,
            funded: None,
            input_order: k + 1,
        });
    }
    if items.is_empty() {
        return Err("wsjf CSV has no items".into());
    }
    let have_cd3 = items.iter().all(|i| i.cd3.is_some());
    if by_cd3 && !have_cd3 {
        return Err("--by-cd3 needs cod_per_week and duration_weeks on every item".into());
    }
    let use_cd3 = have_cd3 && (by_cd3 || !relative);
    let key = |i: &Item| if use_cd3 { i.cd3.unwrap_or(0.0) } else { i.wsjf.unwrap_or(0.0) };
    let size = |i: &Item| if use_cd3 { i.duration_weeks.unwrap_or(0.0) } else { i.job_size.unwrap_or(0.0) };
    items.sort_by(|a, b| key(b).total_cmp(&key(a)).then(size(a).total_cmp(&size(b))).then(a.input_order.cmp(&b.input_order)));
    let mut cum = 0.0;
    for (n, i) in items.iter_mut().enumerate() {
        i.rank = n + 1;
        if let Some(c) = i.cost {
            cum += c;
            i.cumulative_cost = Some(cum);
            i.funded = budget.map(|b| cum <= b + 1e-9);
        }
    }
    let delay = |order: &[&Item]| {
        let mut t = 0.0;
        let mut tot = 0.0;
        for i in order {
            t += i.duration_weeks?;
            tot += i.cod_per_week? * t;
        }
        Some(tot)
    };
    let ranked: Vec<&Item> = items.iter().collect();
    let mut inorder = ranked.clone();
    inorder.sort_by_key(|i| i.input_order);
    Ok(Ranking {
        method: if use_cd3 { "cd3" } else { "wsjf" }.into(),
        delay_cost_ranked: if have_cd3 { delay(&ranked) } else { None },
        delay_cost_input_order: if have_cd3 { delay(&inorder) } else { None },
        items,
        budget,
        warnings,
    })
}

pub fn render_text(r: &Ranking) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{}",
        if r.method == "cd3" {
            "Ranked by CD3 = cost of delay per week / duration (weeks)"
        } else {
            "Ranked by WSJF = (BV + TC + RR/OE) / job size"
        }
    );
    let _ = writeln!(
        s,
        "{:>4} {:<10} {:<34} {:>5} {:>5} {:>7} {:>10} {:>12}  Funded",
        "Rank", "ID", "Name", "CoD", "Size", "WSJF", "CD3", "Cum cost"
    );
    for i in &r.items {
        let f = |v: Option<f64>, p: usize| v.map(|x| format!("{x:.p$}")).unwrap_or_else(|| "-".into());
        let funded = match i.funded {
            Some(true) => "yes",
            Some(false) => "NO (over budget)",
            None => "",
        };
        let _ = writeln!(
            s,
            "{:>4} {:<10} {:<34} {:>5} {:>5} {:>7} {:>10} {:>12}  {funded}",
            i.rank,
            i.id.chars().take(10).collect::<String>(),
            i.name.chars().take(34).collect::<String>(),
            f(i.cost_of_delay, 0),
            f(i.job_size, 0),
            f(i.wsjf, 2),
            f(i.cd3, 0),
            f(i.cumulative_cost, 0)
        );
    }
    if let (Some(a), Some(b)) = (r.delay_cost_ranked, r.delay_cost_input_order) {
        let _ = writeln!(s, "Total cost of delay delivering one at a time: ranked order {a:.0}, input order {b:.0} (saves {:.0})", b - a);
    }
    if let Some(b) = r.budget {
        let _ = writeln!(
            s,
            "Budget {b:.0}: {} of {} item(s) funded in rank order",
            r.items.iter().filter(|i| i.funded == Some(true)).count(),
            r.items.len()
        );
    }
    for w in &r.warnings {
        let _ = writeln!(s, "  note: {w}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsjf_and_cd3() {
        let csv = "id,name,business_value,time_criticality,risk_reduction,job_size,cost,cod_per_week,duration_weeks\n\
            F1,Big,20,8,5,13,500,10000,10\nF2,Quick win,8,13,3,3,100,8000,2\nF3,Small,3,2,1,1,50,1000,1\nF4,Odd,7,1,1,5,400,500,4\n";
        let r = rank(csv, Some(700.0), false).unwrap();
        assert_eq!(r.method, "wsjf");
        let ids: Vec<&str> = r.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["F2", "F3", "F1", "F4"]); // 8.0, 6.0, 2.54, 1.8
        assert_eq!(r.items[2].funded, Some(true));
        assert_eq!(r.items[3].funded, Some(false), "cum 1050 > 700");
        assert!(r.warnings.iter().any(|w| w.contains("F4: business_value 7")));
        let c = rank(csv, None, true).unwrap();
        assert_eq!(c.method, "cd3");
        assert_eq!(c.items[0].id, "F2"); // 4000/wk
        assert!(c.delay_cost_ranked.unwrap() <= c.delay_cost_input_order.unwrap());
        assert!(rank("id,business_value\nA,3\n", None, false).is_err());
        assert!(render_text(&r).contains("Quick win"));
    }
}
