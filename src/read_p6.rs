//! Readers for Oracle Primavera P6 files: XER (tab-delimited export) and P6 XML (PMXML / `APIBusinessObjects`).
//!
//! Both formats are parsed into a small intermediate structure ([`P6Data`]) and then mapped onto the
//! engine-neutral [`Plan`]: WBS nodes become summary tasks, activities become tasks below them, relationships
//! become predecessor links, and P6 calendars become calendars with working times and exceptions.
use crate::model::*;
use chrono::{Datelike, Duration as CDur, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use std::collections::{HashMap, HashSet};
use std::path::Path;

// ---------------------------------------------------------------------------------------------------------
// Intermediate structures
// ---------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct P6Project {
    pub key: String,
    pub short_name: String,
    pub name: String,
    pub start: Option<NaiveDateTime>,
    pub finish: Option<NaiveDateTime>,
    pub data_date: Option<NaiveDateTime>,
    pub calendar: Option<String>,
    /// WBS id of the project's own root node (XER `proj_node_flag=Y`, XML `Project/WBSObjectId`).
    pub root_wbs: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct P6Wbs {
    id: String,
    parent: Option<String>,
    project: String,
    code: String,
    name: String,
    seq: i64,
    is_root: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ActKind {
    #[default]
    Task,
    StartMilestone,
    FinishMilestone,
}

#[derive(Debug, Clone, Default)]
struct P6Task {
    id: String,
    project: String,
    wbs: Option<String>,
    calendar: Option<String>,
    code: String,
    name: String,
    kind: ActKind,
    complete: bool,
    start: Option<NaiveDateTime>,
    finish: Option<NaiveDateTime>,
    act_start: Option<NaiveDateTime>,
    act_finish: Option<NaiveDateTime>,
    baseline_start: Option<NaiveDateTime>,
    baseline_finish: Option<NaiveDateTime>,
    dur_hours: f64,
    pct: f64,
    total_float: Option<f64>,
    cstr_type: Option<i32>,
    cstr_date: Option<NaiveDateTime>,
}

#[derive(Debug, Clone)]
struct P6Rel {
    succ: String,
    pred: String,
    ty: &'static str,
    lag_hours: f64,
}

#[derive(Debug, Clone, Default)]
struct P6Rsrc {
    id: String,
    short_name: Option<String>,
    name: String,
    rtype: &'static str,
    calendar: Option<String>,
    email: Option<String>,
    rate: Option<f64>,
}

#[derive(Debug, Clone, Default)]
struct P6Assn {
    id: String,
    task: String,
    rsrc: String,
    units_pct: f64,
    work_hours: f64,
    cost: Option<f64>,
    start: Option<NaiveDateTime>,
    finish: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Default)]
struct P6Cal {
    id: String,
    name: String,
    base: Option<String>,
    project: Option<String>,
    hours_per_day: Option<f64>,
    hours_per_week: Option<f64>,
    days: Option<[DayRec; 7]>,
    exceptions: Vec<ExceptionRec>,
}

#[derive(Debug, Clone, Default)]
pub struct P6Data {
    format: String,
    pub projects: Vec<P6Project>,
    wbs: Vec<P6Wbs>,
    tasks: Vec<P6Task>,
    rels: Vec<P6Rel>,
    cals: Vec<P6Cal>,
    rsrcs: Vec<P6Rsrc>,
    assns: Vec<P6Assn>,
}

// ---------------------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------------------

fn parse_dt(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    for f in ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(d) = NaiveDateTime::parse_from_str(s, f) {
            return Some(d);
        }
    }
    NaiveDate::parse_from_str(&s[..s.len().min(10)], "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0))
}

fn parse_f(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() { None } else { s.parse().ok() }
}

/// P6 writes working periods ending at `hh:59`; Microsoft Project uses the next whole minute.
fn norm_end(t: NaiveTime) -> NaiveTime {
    if t.minute() == 59 { t + CDur::minutes(1) } else { t }
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").or_else(|_| NaiveTime::parse_from_str(s.trim(), "%H:%M:%S")).ok()
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

fn constraint_from_xer(s: &str) -> Option<i32> {
    Some(match s.trim() {
        "CS_ALAP" => 1,
        "CS_MSO" | "CS_MANDSTART" => 2,
        "CS_MEO" | "CS_MANDFIN" => 3,
        "CS_MSOA" => 4,
        "CS_MSOB" => 5,
        "CS_MEOA" => 6,
        "CS_MEOB" => 7,
        _ => return None,
    })
}

fn constraint_from_xml(s: &str) -> Option<i32> {
    Some(match s.trim() {
        "As Late As Possible" => 1,
        "Start On" | "Mandatory Start" => 2,
        "Finish On" | "Mandatory Finish" => 3,
        "Start On or After" => 4,
        "Start On or Before" => 5,
        "Finish On or After" => 6,
        "Finish On or Before" => 7,
        _ => return None,
    })
}

fn rel_type(s: &str) -> &'static str {
    match s.trim() {
        "PR_SS" | "Start to Start" => "SS",
        "PR_FF" | "Finish to Finish" => "FF",
        "PR_SF" | "Start to Finish" => "SF",
        _ => "FS",
    }
}

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Detect P6 content: `Some(true)` = XER, `Some(false)` = P6 XML.
pub fn detect_bytes(bytes: &[u8]) -> Option<bool> {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("ERMHDR") {
        return Some(true);
    }
    if head.contains("<APIBusinessObjects") {
        return Some(false);
    }
    None
}

fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
    }
}

// ---------------------------------------------------------------------------------------------------------
// XER
// ---------------------------------------------------------------------------------------------------------

/// Parsed XER tables: table name -> rows of (field -> value).
pub type XerTables = HashMap<String, Vec<HashMap<String, String>>>;

/// Split an XER file into tables.
pub fn parse_xer_tables(text: &str) -> Result<(String, XerTables), String> {
    let mut tables: XerTables = HashMap::new();
    let mut version = String::new();
    let (mut table, mut fields): (Option<String>, Vec<String>) = (None, vec![]);
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end_matches(['\r', '\n']);
        let mut cols = line.split('\t');
        match cols.next() {
            Some("ERMHDR") => version = cols.next().unwrap_or("").to_string(),
            Some("%T") => {
                let t = cols.next().unwrap_or("").trim().to_string();
                tables.entry(t.clone()).or_default();
                table = Some(t);
                fields.clear();
            }
            Some("%F") => fields = cols.map(|c| c.trim().to_string()).collect(),
            Some("%R") => {
                let Some(t) = &table else { return Err(format!("XER line {}: record before any %T table", n + 1)) };
                let row = fields.iter().cloned().zip(cols.map(str::to_string)).collect();
                tables.get_mut(t).expect("table exists").push(row);
            }
            Some("%E") => break,
            _ => {}
        }
    }
    if version.is_empty() {
        return Err("not an XER file (missing ERMHDR header)".into());
    }
    Ok((version, tables))
}

/// A node of P6's `clndr_data` mini-language: `(0||Name(attr|val|attr|val)(children...))`.
#[derive(Debug, Default)]
struct CNode {
    name: String,
    attrs: Vec<String>,
    children: Vec<CNode>,
}

impl CNode {
    fn child(&self, name: &str) -> Option<&CNode> {
        self.children.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.chunks(2).find(|c| c[0] == key).and_then(|c| c.get(1)).map(|s| s.as_str())
    }
}

fn parse_cnode(s: &[u8], pos: &mut usize) -> Option<CNode> {
    // expects '(' at pos
    if s.get(*pos) != Some(&b'(') {
        return None;
    }
    *pos += 1;
    let start = *pos;
    while *pos < s.len() && s[*pos] != b'(' && s[*pos] != b')' {
        *pos += 1;
    }
    let header = String::from_utf8_lossy(&s[start..*pos]).to_string();
    let name = header.rsplit("||").next().unwrap_or("").trim().to_string();
    let mut node = CNode { name, ..Default::default() };
    // attribute group
    if s.get(*pos) == Some(&b'(') {
        *pos += 1;
        let a = *pos;
        while *pos < s.len() && s[*pos] != b')' {
            *pos += 1;
        }
        let attrs = String::from_utf8_lossy(&s[a..*pos]).to_string();
        node.attrs = if attrs.is_empty() { vec![] } else { attrs.split('|').map(|x| x.trim().to_string()).collect() };
        *pos += 1;
    }
    // children group
    if s.get(*pos) == Some(&b'(') {
        *pos += 1;
        while s.get(*pos) == Some(&b'(') {
            node.children.push(parse_cnode(s, pos)?);
        }
        if s.get(*pos) == Some(&b')') {
            *pos += 1;
        }
    }
    if s.get(*pos) == Some(&b')') {
        *pos += 1;
    }
    Some(node)
}

fn ranges_of(node: &CNode) -> Vec<(NaiveTime, NaiveTime)> {
    node.children.iter().filter_map(|c| Some((parse_time(c.attr("s")?)?, norm_end(parse_time(c.attr("f")?)?)))).collect()
}

/// Parse XER `clndr_data` into weekdays (Sun..Sat) and exceptions.
fn parse_clndr_data(data: &str) -> (Option<[DayRec; 7]>, Vec<ExceptionRec>) {
    let clean: Vec<u8> = data.bytes().filter(|b| !b.is_ascii_whitespace() && *b != 0x7f).collect();
    let mut pos = 0;
    let Some(root) = parse_cnode(&clean, &mut pos) else { return (None, vec![]) };
    let days = root.child("DaysOfWeek").map(|dow| {
        let mut days: [DayRec; 7] = Default::default();
        for (i, d) in days.iter_mut().enumerate() {
            let r = dow.child(&(i + 1).to_string()).map(ranges_of).unwrap_or_default();
            *d = DayRec { working: Some(!r.is_empty()), ranges: r };
        }
        days
    });
    let base = NaiveDate::from_ymd_opt(1899, 12, 30).expect("valid date");
    let mut exceptions = vec![];
    if let Some(ex) = root.child("Exceptions") {
        for e in &ex.children {
            let Some(serial) = e.attr("d").and_then(|d| d.parse::<i64>().ok()) else { continue };
            let date = base + CDur::days(serial);
            let r = ranges_of(e);
            exceptions.push(ExceptionRec { name: None, from: date, to: date, working: !r.is_empty(), ranges: r });
        }
    }
    exceptions.sort_by_key(|e| e.from);
    (days, exceptions)
}

fn read_xer(text: &str) -> Result<P6Data, String> {
    let (version, t) = parse_xer_tables(text)?;
    let rows = |name: &str| t.get(name).map(|v| v.as_slice()).unwrap_or(&[]);
    let g = |r: &HashMap<String, String>, k: &str| r.get(k).map(|s| s.trim().to_string()).unwrap_or_default();
    let mut d = P6Data { format: format!("P6 XER (v{version})"), ..Default::default() };
    let wbs_rows = rows("PROJWBS");
    for r in rows("PROJECT") {
        let key = g(r, "proj_id");
        let root = wbs_rows.iter().find(|w| g(w, "proj_id") == key && g(w, "proj_node_flag") == "Y");
        let short = g(r, "proj_short_name");
        d.projects.push(P6Project {
            name: root.map(|w| g(w, "wbs_name")).filter(|s| !s.is_empty()).unwrap_or_else(|| short.clone()),
            short_name: short,
            start: parse_dt(&g(r, "plan_start_date")),
            finish: parse_dt(&g(r, "scd_end_date")).or(parse_dt(&g(r, "plan_end_date"))),
            data_date: parse_dt(&g(r, "last_recalc_date")),
            calendar: non_empty(&g(r, "clndr_id")),
            root_wbs: root.map(|w| g(w, "wbs_id")),
            key,
        });
    }
    for r in wbs_rows {
        d.wbs.push(P6Wbs {
            id: g(r, "wbs_id"),
            parent: non_empty(&g(r, "parent_wbs_id")),
            project: g(r, "proj_id"),
            code: g(r, "wbs_short_name"),
            name: g(r, "wbs_name"),
            seq: g(r, "seq_num").parse().unwrap_or(0),
            is_root: g(r, "proj_node_flag") == "Y",
        });
    }
    for r in rows("TASK") {
        let status = g(r, "status_code");
        let complete = status == "TK_Complete";
        let target = parse_f(&g(r, "target_drtn_hr_cnt")).unwrap_or(0.0);
        let remain = parse_f(&g(r, "remain_drtn_hr_cnt"));
        let pct = if complete {
            100.0
        } else {
            match g(r, "complete_pct_type").as_str() {
                "CP_Phys" => parse_f(&g(r, "phys_complete_pct")).unwrap_or(0.0),
                "CP_Units" => {
                    let a = parse_f(&g(r, "act_work_qty")).unwrap_or(0.0);
                    let rem = parse_f(&g(r, "remain_work_qty")).unwrap_or(0.0);
                    if a + rem > 0.0 { a / (a + rem) * 100.0 } else { 0.0 }
                }
                _ if status == "TK_NotStart" => 0.0,
                _ => {
                    if target > 0.0 {
                        ((target - remain.unwrap_or(target)) / target * 100.0).clamp(0.0, 100.0)
                    } else {
                        0.0
                    }
                }
            }
        };
        let kind = match g(r, "task_type").as_str() {
            "TT_Mile" => ActKind::StartMilestone,
            "TT_FinMile" => ActKind::FinishMilestone,
            _ => ActKind::Task,
        };
        let act_start = parse_dt(&g(r, "act_start_date"));
        let act_finish = parse_dt(&g(r, "act_end_date"));
        let start =
            act_start.or(parse_dt(&g(r, "restart_date"))).or(parse_dt(&g(r, "early_start_date"))).or(parse_dt(&g(r, "target_start_date")));
        let finish =
            act_finish.or(parse_dt(&g(r, "reend_date"))).or(parse_dt(&g(r, "early_end_date"))).or(parse_dt(&g(r, "target_end_date")));
        let cstr_type = constraint_from_xer(&g(r, "cstr_type"));
        d.tasks.push(P6Task {
            id: g(r, "task_id"),
            project: g(r, "proj_id"),
            wbs: non_empty(&g(r, "wbs_id")),
            calendar: non_empty(&g(r, "clndr_id")),
            code: g(r, "task_code"),
            name: g(r, "task_name"),
            kind,
            complete,
            start,
            finish,
            act_start,
            act_finish,
            baseline_start: None,
            baseline_finish: None,
            dur_hours: if status == "TK_NotStart" { remain.unwrap_or(target) } else { target },
            pct,
            total_float: parse_f(&g(r, "total_float_hr_cnt")),
            cstr_type,
            cstr_date: if cstr_type.unwrap_or(0) >= 2 { parse_dt(&g(r, "cstr_date")) } else { None },
        });
    }
    for r in rows("TASKPRED") {
        d.rels.push(P6Rel {
            succ: g(r, "task_id"),
            pred: g(r, "pred_task_id"),
            ty: rel_type(&g(r, "pred_type")),
            lag_hours: parse_f(&g(r, "lag_hr_cnt")).unwrap_or(0.0),
        });
    }
    for r in rows("CALENDAR") {
        let (days, exceptions) = parse_clndr_data(&g(r, "clndr_data"));
        d.cals.push(P6Cal {
            id: g(r, "clndr_id"),
            name: g(r, "clndr_name"),
            base: non_empty(&g(r, "base_clndr_id")),
            project: non_empty(&g(r, "proj_id")),
            hours_per_day: parse_f(&g(r, "day_hr_cnt")),
            hours_per_week: parse_f(&g(r, "week_hr_cnt")),
            days,
            exceptions,
        });
    }
    let mut rates: HashMap<String, (String, f64)> = HashMap::new();
    for r in rows("RSRCRATE") {
        if let Some(v) = parse_f(&g(r, "cost_per_qty")) {
            let (id, start) = (g(r, "rsrc_id"), g(r, "start_date"));
            let e = rates.entry(id).or_insert((start.clone(), v));
            if start < e.0 {
                *e = (start, v);
            }
        }
    }
    for r in rows("RSRC") {
        let id = g(r, "rsrc_id");
        d.rsrcs.push(P6Rsrc {
            rate: rates.get(&id).map(|r| r.1),
            short_name: non_empty(&g(r, "rsrc_short_name")),
            name: g(r, "rsrc_name"),
            rtype: if g(r, "rsrc_type") == "RT_Mat" { "Material" } else { "Work" },
            calendar: non_empty(&g(r, "clndr_id")),
            email: non_empty(&g(r, "email_addr")),
            id,
        });
    }
    for r in rows("TASKRSRC") {
        let act = parse_f(&g(r, "act_reg_cost")).unwrap_or(0.0) + parse_f(&g(r, "act_ot_cost")).unwrap_or(0.0);
        let cost = parse_f(&g(r, "remain_cost")).map(|rem| rem + act).or(parse_f(&g(r, "target_cost")));
        d.assns.push(P6Assn {
            id: g(r, "taskrsrc_id"),
            task: g(r, "task_id"),
            rsrc: g(r, "rsrc_id"),
            units_pct: parse_f(&g(r, "target_qty_per_hr")).unwrap_or(1.0) * 100.0,
            work_hours: parse_f(&g(r, "target_qty")).unwrap_or(0.0),
            cost,
            start: parse_dt(&g(r, "act_start_date")).or(parse_dt(&g(r, "restart_date"))).or(parse_dt(&g(r, "target_start_date"))),
            finish: parse_dt(&g(r, "act_end_date")).or(parse_dt(&g(r, "reend_date"))).or(parse_dt(&g(r, "target_end_date"))),
        });
    }
    Ok(d)
}

// ---------------------------------------------------------------------------------------------------------
// P6 XML (APIBusinessObjects)
// ---------------------------------------------------------------------------------------------------------

fn ct<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> String {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name).and_then(|c| c.text()).unwrap_or("").trim().to_string()
}

fn xml_cal(n: roxmltree::Node) -> P6Cal {
    let mut days: Option<[DayRec; 7]> = None;
    if let Some(week) = n.children().find(|c| c.tag_name().name() == "StandardWorkWeek") {
        let mut arr: [DayRec; 7] = Default::default();
        for d in arr.iter_mut() {
            *d = DayRec { working: Some(false), ranges: vec![] };
        }
        for h in week.children().filter(|c| c.tag_name().name() == "StandardWorkHours") {
            let Some(i) = WEEKDAYS.iter().position(|w| *w == ct(h, "DayOfWeek")) else { continue };
            let r: Vec<_> = h
                .children()
                .filter(|c| c.tag_name().name() == "WorkTime")
                .filter_map(|w| Some((parse_time(&ct(w, "Start"))?, norm_end(parse_time(&ct(w, "Finish"))?))))
                .collect();
            arr[i] = DayRec { working: Some(!r.is_empty()), ranges: r };
        }
        days = Some(arr);
    }
    let mut exceptions = vec![];
    if let Some(hx) = n.children().find(|c| c.tag_name().name() == "HolidayOrExceptions") {
        for e in hx.children().filter(|c| c.tag_name().name() == "HolidayOrException") {
            let Some(date) = parse_dt(&ct(e, "Date")).map(|d| d.date()) else { continue };
            let r: Vec<_> = e
                .children()
                .filter(|c| c.tag_name().name() == "WorkTime")
                .filter_map(|w| Some((parse_time(&ct(w, "Start"))?, norm_end(parse_time(&ct(w, "Finish"))?))))
                .collect();
            exceptions.push(ExceptionRec { name: None, from: date, to: date, working: !r.is_empty(), ranges: r });
        }
    }
    exceptions.sort_by_key(|e| e.from);
    P6Cal {
        id: ct(n, "ObjectId"),
        name: ct(n, "Name"),
        base: non_empty(&ct(n, "BaseCalendarObjectId")),
        project: non_empty(&ct(n, "ProjectObjectId")),
        hours_per_day: parse_f(&ct(n, "HoursPerDay")),
        hours_per_week: parse_f(&ct(n, "HoursPerWeek")),
        days,
        exceptions,
    }
}

fn read_p6_xml(text: &str) -> Result<P6Data, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("invalid P6 XML: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "APIBusinessObjects" {
        return Err("not a P6 XML file (root element is not APIBusinessObjects)".into());
    }
    let mut d = P6Data { format: "P6 XML".into(), ..Default::default() };
    let elems = |name: &'static str| root.descendants().filter(move |n| n.is_element() && n.tag_name().name() == name);
    for p in root.children().filter(|n| n.tag_name().name() == "Project") {
        let key = ct(p, "ObjectId");
        d.projects.push(P6Project {
            short_name: ct(p, "Id"),
            name: non_empty(&ct(p, "Name")).unwrap_or_else(|| ct(p, "Id")),
            start: parse_dt(&ct(p, "PlannedStartDate")).or(parse_dt(&ct(p, "StartDate"))),
            finish: parse_dt(&ct(p, "ScheduledFinishDate")).or(parse_dt(&ct(p, "FinishDate"))),
            data_date: parse_dt(&ct(p, "DataDate")),
            calendar: non_empty(&ct(p, "ActivityDefaultCalendarObjectId")),
            root_wbs: non_empty(&ct(p, "WBSObjectId")),
            key: key.clone(),
        });
        // Activities etc. nested in the project may omit ProjectObjectId; stamp it from the parent.
        for a in p.descendants().filter(|n| n.tag_name().name() == "Activity") {
            d.tasks.push(xml_task(a, &key));
        }
        for w in p.descendants().filter(|n| n.tag_name().name() == "WBS") {
            d.wbs.push(xml_wbs(w, &key));
        }
    }
    let seen_tasks: HashSet<String> = d.tasks.iter().map(|t| t.id.clone()).collect();
    for a in elems("Activity").filter(|a| !seen_tasks.contains(&ct(*a, "ObjectId"))) {
        let proj = ct(a, "ProjectObjectId");
        d.tasks.push(xml_task(a, &proj));
    }
    let seen_wbs: HashSet<String> = d.wbs.iter().map(|w| w.id.clone()).collect();
    for w in elems("WBS").filter(|w| !seen_wbs.contains(&ct(*w, "ObjectId"))) {
        let proj = ct(w, "ProjectObjectId");
        d.wbs.push(xml_wbs(w, &proj));
    }
    for r in elems("Relationship") {
        d.rels.push(P6Rel {
            succ: ct(r, "SuccessorActivityObjectId"),
            pred: ct(r, "PredecessorActivityObjectId"),
            ty: rel_type(&ct(r, "Type")),
            lag_hours: parse_f(&ct(r, "Lag")).unwrap_or(0.0),
        });
    }
    for c in elems("Calendar") {
        d.cals.push(xml_cal(c));
    }
    let mut rates: HashMap<String, (String, f64)> = HashMap::new();
    for r in elems("ResourceRate") {
        if let Some(v) = parse_f(&ct(r, "PricePerUnit")) {
            let (id, start) = (ct(r, "ResourceObjectId"), ct(r, "EffectiveDate"));
            let e = rates.entry(id).or_insert((start.clone(), v));
            if start < e.0 {
                *e = (start, v);
            }
        }
    }
    for r in elems("Resource") {
        let id = ct(r, "ObjectId");
        d.rsrcs.push(P6Rsrc {
            rate: rates.get(&id).map(|r| r.1),
            short_name: non_empty(&ct(r, "Id")),
            name: ct(r, "Name"),
            rtype: if ct(r, "ResourceType") == "Material" { "Material" } else { "Work" },
            calendar: non_empty(&ct(r, "CalendarObjectId")),
            email: non_empty(&ct(r, "EmailAddress")),
            id,
        });
    }
    for a in elems("ResourceAssignment") {
        d.assns.push(P6Assn {
            id: ct(a, "ObjectId"),
            task: ct(a, "ActivityObjectId"),
            rsrc: ct(a, "ResourceObjectId"),
            units_pct: parse_f(&ct(a, "PlannedUnitsPerTime")).unwrap_or(1.0) * 100.0,
            work_hours: parse_f(&ct(a, "AtCompletionUnits")).or(parse_f(&ct(a, "PlannedUnits"))).unwrap_or(0.0),
            cost: parse_f(&ct(a, "AtCompletionCost")).or(parse_f(&ct(a, "PlannedCost"))),
            start: parse_dt(&ct(a, "StartDate")).or(parse_dt(&ct(a, "PlannedStartDate"))),
            finish: parse_dt(&ct(a, "FinishDate")).or(parse_dt(&ct(a, "PlannedFinishDate"))),
        });
    }
    Ok(d)
}

fn xml_wbs(w: roxmltree::Node, project: &str) -> P6Wbs {
    P6Wbs {
        id: ct(w, "ObjectId"),
        parent: non_empty(&ct(w, "ParentObjectId")),
        project: non_empty(&ct(w, "ProjectObjectId")).unwrap_or_else(|| project.to_string()),
        code: ct(w, "Code"),
        name: ct(w, "Name"),
        seq: ct(w, "SequenceNumber").parse().unwrap_or(0),
        is_root: false,
    }
}

fn xml_task(a: roxmltree::Node, project: &str) -> P6Task {
    let status = ct(a, "Status");
    let complete = status == "Completed";
    let kind = match ct(a, "Type").as_str() {
        "Start Milestone" => ActKind::StartMilestone,
        "Finish Milestone" => ActKind::FinishMilestone,
        _ => ActKind::Task,
    };
    let pct = parse_f(&ct(a, "PercentComplete")).map(|v| v * 100.0).unwrap_or(0.0);
    let cstr_type = constraint_from_xml(&ct(a, "PrimaryConstraintType"));
    let act_start = parse_dt(&ct(a, "ActualStartDate"));
    let act_finish = parse_dt(&ct(a, "ActualFinishDate"));
    P6Task {
        id: ct(a, "ObjectId"),
        project: non_empty(&ct(a, "ProjectObjectId")).unwrap_or_else(|| project.to_string()),
        wbs: non_empty(&ct(a, "WBSObjectId")),
        calendar: non_empty(&ct(a, "CalendarObjectId")),
        code: ct(a, "Id"),
        name: ct(a, "Name"),
        kind,
        complete,
        start: act_start
            .or(parse_dt(&ct(a, "StartDate")))
            .or(parse_dt(&ct(a, "RemainingEarlyStartDate")))
            .or(parse_dt(&ct(a, "PlannedStartDate"))),
        finish: act_finish
            .or(parse_dt(&ct(a, "FinishDate")))
            .or(parse_dt(&ct(a, "RemainingEarlyFinishDate")))
            .or(parse_dt(&ct(a, "PlannedFinishDate"))),
        act_start,
        act_finish,
        baseline_start: None,
        baseline_finish: None,
        dur_hours: parse_f(&ct(a, "AtCompletionDuration")).or(parse_f(&ct(a, "PlannedDuration"))).unwrap_or(0.0),
        pct: if complete { 100.0 } else { pct.clamp(0.0, 100.0) },
        total_float: parse_f(&ct(a, "TotalFloat")),
        cstr_type,
        cstr_date: if cstr_type.unwrap_or(0) >= 2 { parse_dt(&ct(a, "PrimaryConstraintDate")) } else { None },
    }
}

// ---------------------------------------------------------------------------------------------------------
// Mapping to Plan
// ---------------------------------------------------------------------------------------------------------

/// Parse a P6 file (XER or XML) into its intermediate form.
pub fn parse_file(path: &Path) -> Result<P6Data, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let is_xer = match detect_bytes(&bytes) {
        Some(x) => x,
        None => path.extension().map(|e| e.eq_ignore_ascii_case("xer")).unwrap_or(false),
    };
    let text = decode(&bytes);
    let res = if is_xer { read_xer(&text) } else { read_p6_xml(&text) };
    res.map_err(|e| format!("{}: {e}", path.display()))
}

/// Choose a project by ObjectId/proj_id, short name (Id) or name (case-insensitive).
pub fn select_project<'a>(data: &'a P6Data, sel: Option<&str>) -> Result<&'a P6Project, String> {
    if data.projects.is_empty() {
        return Err("file contains no projects".into());
    }
    let list = || data.projects.iter().map(|p| format!("  {} | {} | {}", p.key, p.short_name, p.name)).collect::<Vec<_>>().join("\n");
    match sel {
        None => Ok(&data.projects[0]),
        Some(s) => data
            .projects
            .iter()
            .find(|p| p.key == s || p.short_name.eq_ignore_ascii_case(s) || p.name.eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("no project matches '{s}'. Projects in this file (id | short name | name):\n{}", list())),
    }
}

/// Weekday working days between two dates (inclusive), at least 1 when both exist.
fn workdays(a: NaiveDateTime, b: NaiveDateTime) -> f64 {
    if b <= a {
        return 0.0;
    }
    let mut n = 0;
    let mut d = a.date();
    while d <= b.date() {
        if d.weekday().num_days_from_monday() < 5 {
            n += 1;
        }
        d = d.succ_opt().expect("date overflow");
    }
    n.max(1) as f64
}

/// Map one project of a P6 file onto a [`Plan`].
pub fn to_plan(data: &P6Data, sel: Option<&str>) -> Result<Plan, String> {
    let proj = select_project(data, sel)?;
    let pk = &proj.key;
    // Calendars: global/resource calendars plus this project's own.
    let cals: Vec<&P6Cal> = data.cals.iter().filter(|c| c.project.as_ref().is_none_or(|p| p == pk)).collect();
    let cal_uid = |id: &str| id.parse::<i32>().ok();
    let default_cal = proj.calendar.clone().or_else(|| cals.first().map(|c| c.id.clone()));
    let def = default_cal.as_ref().and_then(|id| cals.iter().find(|c| &c.id == id));
    let hpd = def.and_then(|c| c.hours_per_day).filter(|h| *h > 0.0).unwrap_or(8.0);
    let hpw = def.and_then(|c| c.hours_per_week).filter(|h| *h > 0.0).unwrap_or(hpd * 5.0);
    let mut plan = Plan {
        format: data.format.clone(),
        title: Some(proj.name.clone()),
        name: Some(proj.short_name.clone()),
        start: proj.start,
        finish: proj.finish,
        minutes_per_day: (hpd * 60.0).round() as i32,
        minutes_per_week: (hpw * 60.0).round() as i32,
        default_calendar_uid: default_cal.as_deref().and_then(cal_uid),
        ..Default::default()
    };
    let known_cal: HashSet<&str> = cals.iter().map(|c| c.id.as_str()).collect();
    for c in &cals {
        let Some(uid) = cal_uid(&c.id) else { continue };
        let base = c.base.as_deref().filter(|b| known_cal.contains(b)).and_then(cal_uid);
        let days = match (&c.days, base) {
            (Some(d), _) => d.clone(),
            (None, Some(_)) => Default::default(), // inherit everything
            (None, None) => CalendarRec::standard(0).days,
        };
        plan.calendars.push(CalendarRec { uid, name: c.name.clone(), base_uid: base, days, exceptions: c.exceptions.clone() });
    }

    // WBS tree.
    let wbs: Vec<&P6Wbs> = data.wbs.iter().filter(|w| &w.project == pk && !w.is_root && Some(&w.id) != proj.root_wbs.as_ref()).collect();
    let wbs_ids: HashSet<&str> = wbs.iter().map(|w| w.id.as_str()).collect();
    let parent_of = |w: &P6Wbs| w.parent.as_deref().filter(|p| wbs_ids.contains(p)).map(str::to_string);
    let mut wbs_children: HashMap<Option<String>, Vec<&P6Wbs>> = HashMap::new();
    for w in &wbs {
        wbs_children.entry(parent_of(w)).or_default().push(w);
    }
    for v in wbs_children.values_mut() {
        v.sort_by(|a, b| a.seq.cmp(&b.seq).then(a.code.cmp(&b.code)));
    }
    let tasks: Vec<&P6Task> = data.tasks.iter().filter(|t| &t.project == pk).collect();
    let mut acts_in: HashMap<Option<String>, Vec<&P6Task>> = HashMap::new();
    for t in &tasks {
        acts_in.entry(t.wbs.clone().filter(|w| wbs_ids.contains(w.as_str()))).or_default().push(t);
    }
    for v in acts_in.values_mut() {
        v.sort_by(|a, b| a.start.or(a.finish).cmp(&b.start.or(b.finish)).then(a.code.cmp(&b.code)));
    }

    // Depth-first emission: each WBS node, then its activities, then its child WBS nodes.
    enum Item<'a> {
        Wbs(&'a P6Wbs, String),
        Act(&'a P6Task),
    }
    let mut order: Vec<(Item, i32)> = vec![];
    fn walk<'a>(
        node: Option<String>,
        path: String,
        level: i32,
        wc: &HashMap<Option<String>, Vec<&'a P6Wbs>>,
        ai: &HashMap<Option<String>, Vec<&'a P6Task>>,
        out: &mut Vec<(Item<'a>, i32)>,
    ) {
        for a in ai.get(&node).into_iter().flatten() {
            out.push((Item::Act(a), level));
        }
        for w in wc.get(&node).into_iter().flatten() {
            let p = if path.is_empty() { w.code.clone() } else { format!("{path}.{}", w.code) };
            out.push((Item::Wbs(w, p.clone()), level));
            walk(Some(w.id.clone()), p, level + 1, wc, ai, out);
        }
    }
    walk(None, String::new(), 1, &wbs_children, &acts_in, &mut order);

    let mut uid_of_task: HashMap<&str, i32> = HashMap::new();
    for (i, (item, _)) in order.iter().enumerate() {
        if let Item::Act(a) = item {
            uid_of_task.insert(a.id.as_str(), i as i32 + 1);
        }
    }
    let mut preds: HashMap<&str, Vec<Pred>> = HashMap::new();
    for r in &data.rels {
        let (Some(_), Some(&puid)) = (uid_of_task.get(r.succ.as_str()), uid_of_task.get(r.pred.as_str())) else { continue };
        let days = r.lag_hours / hpd;
        let disp = if (days - days.round()).abs() < 1e-9 { format!("{}d", days.round() as i64) } else { format!("{}h", r.lag_hours) };
        preds.entry(r.succ.as_str()).or_default().push(Pred {
            predecessor_uid: puid,
            predecessor_id: Some(puid),
            link_type: r.ty.into(),
            lag_hours: r.lag_hours,
            lag_display: disp,
        });
    }
    for (i, (item, level)) in order.iter().enumerate() {
        let uid = i as i32 + 1;
        match item {
            Item::Act(a) => {
                let ms = a.kind != ActKind::Task;
                let (start, finish) = match a.kind {
                    ActKind::StartMilestone => (a.start.or(a.finish), a.start.or(a.finish)),
                    ActKind::FinishMilestone => (a.finish.or(a.start), a.finish.or(a.start)),
                    ActKind::Task => (a.start, a.finish),
                };
                plan.tasks.push(TaskRec {
                    uid,
                    id: Some(uid),
                    name: a.name.clone(),
                    outline_level: *level,
                    activity_id: non_empty(&a.code),
                    start,
                    finish,
                    duration_hours: if ms { 0.0 } else { a.dur_hours },
                    percent_complete: a.pct,
                    milestone: ms,
                    critical: !a.complete && a.total_float.is_some_and(|f| f <= 0.0),
                    predecessors: preds.remove(a.id.as_str()).unwrap_or_default(),
                    total_slack_hours: a.total_float,
                    constraint_type: a.cstr_type,
                    constraint_date: a.cstr_date,
                    actual_start: a.act_start,
                    actual_finish: a.act_finish,
                    baseline_start: a.baseline_start,
                    baseline_finish: a.baseline_finish,
                    calendar_uid: a.calendar.as_deref().filter(|c| known_cal.contains(c)).and_then(cal_uid),
                    ..Default::default()
                });
            }
            Item::Wbs(w, path) => {
                plan.tasks.push(TaskRec {
                    uid,
                    id: Some(uid),
                    name: w.name.clone(),
                    outline_level: *level,
                    wbs: Some(path.clone()),
                    summary: true,
                    ..Default::default()
                });
            }
        }
    }
    // Roll up summary dates, % complete and criticality (deepest first).
    let n = plan.tasks.len();
    for i in (0..n).rev() {
        if !plan.tasks[i].summary {
            continue;
        }
        let lvl = plan.tasks[i].outline_level;
        let kids: Vec<usize> =
            (i + 1..n).take_while(|&k| plan.tasks[k].outline_level > lvl).filter(|&k| plan.tasks[k].outline_level == lvl + 1).collect();
        let start = kids.iter().filter_map(|&k| plan.tasks[k].start).min();
        let finish = kids.iter().filter_map(|&k| plan.tasks[k].finish).max();
        let (mut w, mut wp) = (0.0, 0.0);
        for &k in &kids {
            w += plan.tasks[k].duration_hours;
            wp += plan.tasks[k].duration_hours * plan.tasks[k].percent_complete;
        }
        let crit = kids.iter().any(|&k| plan.tasks[k].critical);
        let t = &mut plan.tasks[i];
        t.start = start;
        t.finish = finish;
        t.duration_hours = match (start, finish) {
            (Some(a), Some(b)) => workdays(a, b) * hpd,
            _ => 0.0,
        };
        t.percent_complete = if w > 0.0 { (wp / w).round() } else { 0.0 }; // MSPDI stores whole percents
        t.critical = crit;
    }

    // Resources (only those used by this project, or all if the project has no assignments).
    let assns: Vec<&P6Assn> = data.assns.iter().filter(|a| uid_of_task.contains_key(a.task.as_str())).collect();
    let used: HashSet<&str> = assns.iter().map(|a| a.rsrc.as_str()).collect();
    let mut rsrc_uid: HashMap<&str, i32> = HashMap::new();
    for r in data.rsrcs.iter().filter(|r| used.is_empty() || used.contains(r.id.as_str())) {
        let uid = rsrc_uid.len() as i32 + 1;
        rsrc_uid.insert(r.id.as_str(), uid);
        plan.resources.push(ResourceRec {
            uid,
            id: Some(uid),
            name: r.name.clone(),
            rtype: r.rtype.into(),
            initials: r.short_name.clone(),
            max_units: Some(100.0),
            email: r.email.clone(),
            standard_rate: r.rate,
            calendar_uid: r.calendar.as_deref().filter(|c| known_cal.contains(c)).and_then(cal_uid),
            ..Default::default()
        });
    }
    for (i, a) in assns.iter().enumerate() {
        let (Some(&tu), Some(&ru)) = (uid_of_task.get(a.task.as_str()), rsrc_uid.get(a.rsrc.as_str())) else { continue };
        plan.assignments.push(AssignmentRec {
            uid: a.id.parse().unwrap_or(i as i32 + 1),
            task_uid: tu,
            resource_uid: ru,
            units: a.units_pct,
            work_hours: a.work_hours,
            start: a.start,
            finish: a.finish,
            cost: a.cost,
        });
    }
    // Task costs = sum of assignment costs.
    let mut cost: HashMap<i32, f64> = HashMap::new();
    for a in &plan.assignments {
        if let Some(c) = a.cost {
            *cost.entry(a.task_uid).or_default() += c;
        }
    }
    for t in &mut plan.tasks {
        if let Some(c) = cost.get(&t.uid) {
            t.cost = Some(*c);
        }
    }
    if plan.start.is_none() {
        plan.start = plan.tasks.iter().filter_map(|t| t.start).min();
    }
    if plan.finish.is_none() {
        plan.finish = plan.tasks.iter().filter_map(|t| t.finish).max();
    }
    Ok(plan)
}

/// Read a P6 file and map the selected project (default: the first one). Warns on stderr when the file
/// holds several projects and none was selected.
pub fn read(path: &Path, project: Option<&str>) -> Result<Plan, String> {
    let data = parse_file(path)?;
    if project.is_none() && data.projects.len() > 1 {
        eprintln!(
            "note: {} contains {} projects; using '{}'. Choose another with --project <id|name> (see `mpptool projects`).",
            path.display(),
            data.projects.len(),
            data.projects[0].short_name
        );
    }
    to_plan(&data, project).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clndr_data_parsing() {
        let data = "(0||CalendarData()((0||DaysOfWeek()((0||1()())(0||2()((0||0(s|08:00|f|12:00)())(0||1(s|13:00|f|17:00)())))\
                    (0||3()())(0||4()())(0||5()())(0||6()())(0||7()())))(0||VIEW(ShowTotal|Y)())(0||Exceptions()((0||0(d|46016)())\
                    (0||1(d|46017)((0||0(s|09:00|f|13:00)())))))))";
        let (days, ex) = parse_clndr_data(data);
        let days = days.unwrap();
        assert_eq!(days[0].working, Some(false));
        assert_eq!(days[1].ranges.len(), 2);
        assert_eq!(ex.len(), 2);
        assert_eq!(ex[0].from.to_string(), "2025-12-25");
        assert!(!ex[0].working);
        assert!(ex[1].working && ex[1].ranges[0].0 == NaiveTime::from_hms_opt(9, 0, 0).unwrap());
    }

    #[test]
    fn detection_and_codes() {
        assert_eq!(detect_bytes(b"ERMHDR\t8.2\t"), Some(true));
        assert_eq!(detect_bytes(b"<?xml version=\"1.0\"?><APIBusinessObjects xmlns=\"x\">"), Some(false));
        assert_eq!(detect_bytes(b"<Project/>"), None);
        assert_eq!(constraint_from_xer("CS_MSOA"), Some(4));
        assert_eq!(constraint_from_xml("Finish On or Before"), Some(7));
        assert_eq!(rel_type("PR_SS"), "SS");
        assert_eq!(norm_end(NaiveTime::from_hms_opt(11, 59, 0).unwrap()), NaiveTime::from_hms_opt(12, 0, 0).unwrap());
    }

    #[test]
    fn xer_tables_handle_crlf() {
        let (v, t) = parse_xer_tables("ERMHDR\t20.12\r\n%T\tPROJECT\r\n%F\tproj_id\tproj_short_name\r\n%R\t1\tA\r\n%E\r\n").unwrap();
        assert_eq!(v, "20.12");
        assert_eq!(t["PROJECT"][0]["proj_short_name"], "A");
    }
}
