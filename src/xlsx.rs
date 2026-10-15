//! Excel / Smartsheet-friendly workbook export.
use crate::model::{Plan, TaskRec};
use chrono::{Datelike, NaiveDateTime, Timelike};
use rust_xlsxwriter::{ExcelDateTime, Format, FormatAlign, Workbook, Worksheet, XlsxError};
use std::collections::HashMap;
use std::path::Path;

/// Column headers of the Tasks sheet (also documented in the README).
pub const TASK_COLUMNS: [&str; 18] = [
    "Row",
    "Task Name",
    "Outline Level",
    "WBS",
    "Activity ID",
    "Start",
    "Finish",
    "Duration (days)",
    "% Complete",
    "Predecessors",
    "Assigned To",
    "Milestone",
    "Critical",
    "Summary",
    "Baseline Start",
    "Baseline Finish",
    "ID",
    "Notes",
];

fn xdt(d: NaiveDateTime) -> Result<ExcelDateTime, XlsxError> {
    ExcelDateTime::from_ymd(d.year() as u16, d.month() as u8, d.day() as u8)?.and_hms(d.hour() as u16, d.minute() as u8, 0)
}

/// Predecessors as Smartsheet/Excel row numbers, e.g. `3, 5SS +2d`. Links to tasks not in the sheet are dropped.
pub fn preds_by_row(t: &TaskRec, rows: &HashMap<i32, usize>, hours_per_day: f64) -> String {
    t.predecessors
        .iter()
        .filter_map(|p| {
            let r = rows.get(&p.predecessor_uid)?;
            let mut s = r.to_string();
            if p.link_type != "FS" || p.lag_hours != 0.0 {
                s += &p.link_type;
            }
            if p.lag_hours != 0.0 {
                let d = (p.lag_hours / hours_per_day * 100.0).round() / 100.0;
                s += &format!(" {}{d}d", if d > 0.0 { "+" } else { "" });
            }
            Some(s)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn set_widths(ws: &mut Worksheet, widths: &[f64]) -> Result<(), XlsxError> {
    for (i, w) in widths.iter().enumerate() {
        ws.set_column_width(i as u16, *w)?;
    }
    Ok(())
}

/// The visible rows: every task except the project summary (UID 0) and blank rows.
pub fn sheet_tasks(plan: &Plan) -> Vec<&TaskRec> {
    plan.tasks.iter().filter(|t| !(t.uid == 0 && t.outline_level == 0) && !crate::mspdi_writer::is_null_task(t)).collect()
}

/// Write the workbook: a `Tasks` sheet (hierarchy indentation + Excel row groups, predecessors by row
/// number) and a `Resources` sheet.
pub fn write(plan: &Plan, path: &Path) -> Result<usize, String> {
    build(plan, path).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn build(plan: &Plan, path: &Path) -> Result<usize, XlsxError> {
    let hpd = if plan.minutes_per_day > 0 { plan.minutes_per_day as f64 / 60.0 } else { 8.0 };
    let tasks = sheet_tasks(plan);
    let rows: HashMap<i32, usize> = tasks.iter().enumerate().map(|(i, t)| (t.uid, i + 1)).collect();
    let res_names: HashMap<i32, &str> = plan.resources.iter().map(|r| (r.uid, r.name.as_str())).collect();
    let mut assigned: HashMap<i32, Vec<&str>> = HashMap::new();
    for a in &plan.assignments {
        if let Some(n) = res_names.get(&a.resource_uid) {
            assigned.entry(a.task_uid).or_default().push(n);
        }
    }
    let mut wb = Workbook::new();
    let header = Format::new().set_bold().set_background_color("#DDEBF7").set_border_bottom(rust_xlsxwriter::FormatBorder::Thin);
    let date = Format::new().set_num_format("yyyy-mm-dd");
    let pct = Format::new().set_num_format("0%");
    let num = Format::new().set_num_format("0.##");
    let ws = wb.add_worksheet();
    ws.set_name("Tasks")?;
    for (c, h) in TASK_COLUMNS.iter().enumerate() {
        ws.write_with_format(0, c as u16, *h, &header)?;
    }
    let min_level = tasks.iter().map(|t| t.outline_level).min().unwrap_or(1);
    for (i, t) in tasks.iter().enumerate() {
        let r = (i + 1) as u32;
        let level = (t.outline_level - min_level).clamp(0, 15) as u8;
        let mut name_fmt = Format::new().set_indent(level).set_align(FormatAlign::Left);
        if t.summary {
            name_fmt = name_fmt.set_bold();
        }
        ws.write_number(r, 0, r as f64)?;
        ws.write_with_format(r, 1, t.name.as_str(), &name_fmt)?;
        ws.write_number(r, 2, t.outline_level as f64)?;
        if let Some(w) = &t.wbs {
            ws.write(r, 3, w.as_str())?;
        }
        if let Some(a) = &t.activity_id {
            ws.write(r, 4, a.as_str())?;
        }
        if let Some(d) = t.start {
            ws.write_datetime_with_format(r, 5, xdt(d)?, &date)?;
        }
        if let Some(d) = t.finish {
            ws.write_datetime_with_format(r, 6, xdt(d)?, &date)?;
        }
        ws.write_number_with_format(r, 7, (t.duration_hours / hpd * 100.0).round() / 100.0, &num)?;
        ws.write_number_with_format(r, 8, t.percent_complete / 100.0, &pct)?;
        ws.write(r, 9, preds_by_row(t, &rows, hpd))?;
        if let Some(v) = assigned.get(&t.uid) {
            ws.write(r, 10, v.join(", "))?;
        }
        ws.write_boolean(r, 11, t.milestone)?;
        ws.write_boolean(r, 12, t.critical)?;
        ws.write_boolean(r, 13, t.summary)?;
        if let Some(d) = t.baseline_start {
            ws.write_datetime_with_format(r, 14, xdt(d)?, &date)?;
        }
        if let Some(d) = t.baseline_finish {
            ws.write_datetime_with_format(r, 15, xdt(d)?, &date)?;
        }
        if let Some(id) = t.id {
            ws.write_number(r, 16, id as f64)?;
        }
        if let Some(n) = &t.notes {
            ws.write(r, 17, n.as_str())?;
        }
    }
    // Excel outline groups for summary tasks (collapsible in Excel; Smartsheet ignores them).
    for (i, t) in tasks.iter().enumerate() {
        if !t.summary {
            continue;
        }
        let last = tasks[i + 1..].iter().position(|x| x.outline_level <= t.outline_level).map_or(tasks.len(), |p| i + 1 + p);
        if last > i + 1 && t.outline_level - min_level < 7 {
            ws.group_rows((i + 2) as u32, last as u32)?;
        }
    }
    ws.set_freeze_panes(1, 2)?;
    if !tasks.is_empty() {
        ws.autofilter(0, 0, tasks.len() as u32, (TASK_COLUMNS.len() - 1) as u16)?;
    }
    set_widths(ws, &[5.0, 48.0, 7.0, 10.0, 12.0, 11.0, 11.0, 9.0, 8.0, 18.0, 20.0, 9.0, 8.0, 9.0, 11.0, 11.0, 6.0, 40.0])?;

    let ws = wb.add_worksheet();
    ws.set_name("Resources")?;
    for (c, h) in ["Name", "Type", "Max Units", "Standard Rate", "Email", "Group", "Assignments", "Work (h)"].iter().enumerate() {
        ws.write_with_format(0, c as u16, *h, &header)?;
    }
    for (i, r) in plan.resources.iter().enumerate() {
        let row = (i + 1) as u32;
        let asg: Vec<_> = plan.assignments.iter().filter(|a| a.resource_uid == r.uid).collect();
        ws.write(row, 0, r.name.as_str())?;
        ws.write(row, 1, r.rtype.as_str())?;
        if let Some(m) = r.max_units {
            ws.write_number_with_format(row, 2, m / 100.0, &pct)?;
        }
        if let Some(v) = r.standard_rate {
            ws.write_number(row, 3, v)?;
        }
        if let Some(v) = &r.email {
            ws.write(row, 4, v.as_str())?;
        }
        if let Some(v) = &r.group {
            ws.write(row, 5, v.as_str())?;
        }
        ws.write_number(row, 6, asg.len() as f64)?;
        ws.write_number_with_format(row, 7, asg.iter().map(|a| a.work_hours).sum::<f64>(), &num)?;
    }
    ws.set_freeze_panes(1, 0)?;
    set_widths(ws, &[28.0, 10.0, 10.0, 13.0, 26.0, 16.0, 12.0, 10.0])?;
    wb.save(path)?;
    Ok(tasks.len())
}
