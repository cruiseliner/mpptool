//! Microsoft Project XML (MSPDI) serializer. Element order follows files saved by Project 2010+.
use crate::model::*;
use chrono::{NaiveDateTime, NaiveTime};
use std::collections::HashSet;
use std::fmt::Write;

/// Writer options.
#[derive(Debug, Clone, Default)]
pub struct WriteOptions {
    /// Add Start-No-Earlier-Than constraints to unconstrained, unlinked leaf tasks that start after the project
    /// start, so Microsoft Project keeps their dates when it reschedules.
    pub pin_dates: bool,
}

/// Escape text for XML element content.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => o.push(c),
        }
    }
    o
}

fn dt(d: &NaiveDateTime) -> String {
    d.format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn tm(t: &NaiveTime) -> String {
    t.format("%H:%M:%S").to_string()
}

/// Hours -> xsd:duration as written by Microsoft Project: `PT#H#M#S`.
pub fn pt(hours: f64) -> String {
    let total = (hours.max(0.0) * 3600.0).round() as i64;
    format!("PT{}H{}M{}S", total / 3600, (total % 3600) / 60, total % 60)
}

fn money(v: f64) -> String {
    // MSPDI stores currency in hundredths ("cents") for Cost elements.
    format!("{}", (v * 100.0).round() as i64)
}

struct W {
    o: String,
}

impl W {
    fn tag(&mut self, ind: usize, name: &str, val: impl std::fmt::Display) {
        let _ = writeln!(self.o, "{}<{name}>{val}</{name}>", "\t".repeat(ind));
    }
    fn opt_s(&mut self, ind: usize, name: &str, val: &Option<String>) {
        if let Some(v) = val.as_deref().filter(|v| !v.is_empty()) {
            self.tag(ind, name, esc(v));
        }
    }
    fn opt_dt(&mut self, ind: usize, name: &str, val: &Option<NaiveDateTime>) {
        if let Some(v) = val {
            self.tag(ind, name, dt(v));
        }
    }
    fn open(&mut self, ind: usize, name: &str) {
        let _ = writeln!(self.o, "{}<{name}>", "\t".repeat(ind));
    }
    fn close(&mut self, ind: usize, name: &str) {
        let _ = writeln!(self.o, "{}</{name}>", "\t".repeat(ind));
    }
    fn working_times(&mut self, ind: usize, ranges: &[(NaiveTime, NaiveTime)]) {
        if ranges.is_empty() {
            return;
        }
        self.open(ind, "WorkingTimes");
        for (a, b) in ranges {
            let _ = writeln!(
                self.o,
                "{}<WorkingTime><FromTime>{}</FromTime><ToTime>{}</ToTime></WorkingTime>",
                "\t".repeat(ind + 1),
                tm(a),
                tm(b)
            );
        }
        self.close(ind, "WorkingTimes");
    }
    fn calendar(&mut self, c: &CalendarRec, known: &HashSet<i32>) {
        self.open(2, "Calendar");
        self.tag(3, "UID", c.uid);
        self.tag(3, "Name", esc(&c.name));
        let base = c.base_uid.filter(|b| known.contains(b));
        self.tag(3, "IsBaseCalendar", base.is_none() as i32);
        self.tag(3, "IsBaselineCalendar", 0);
        self.tag(3, "BaseCalendarUID", base.unwrap_or(-1));
        self.open(3, "WeekDays");
        for (i, d) in c.days.iter().enumerate() {
            // Base calendars must define every day; derived calendars only their overrides.
            let working = match (d.working, base) {
                (Some(w), _) => w,
                (None, None) => (1..=5).contains(&i),
                (None, Some(_)) => continue,
            };
            self.open(4, "WeekDay");
            self.tag(5, "DayType", i + 1);
            self.tag(5, "DayWorking", working as i32);
            if working {
                let std_ranges = CalendarRec::standard(0).days[1].ranges.clone();
                self.working_times(5, if d.ranges.is_empty() { &std_ranges } else { &d.ranges });
            }
            self.close(4, "WeekDay");
        }
        // Exceptions in the Project 2003 form (DayType 0), as Project itself still writes them.
        for e in &c.exceptions {
            self.open(4, "WeekDay");
            self.tag(5, "DayType", 0);
            self.tag(5, "DayWorking", e.working as i32);
            self.open(5, "TimePeriod");
            self.tag(6, "FromDate", format!("{}T00:00:00", e.from));
            self.tag(6, "ToDate", format!("{}T23:59:00", e.to));
            self.close(5, "TimePeriod");
            if e.working {
                self.working_times(5, &e.ranges);
            }
            self.close(4, "WeekDay");
        }
        self.close(3, "WeekDays");
        if !c.exceptions.is_empty() {
            self.open(3, "Exceptions");
            for e in &c.exceptions {
                self.open(4, "Exception");
                self.tag(5, "EnteredByOccurrences", 0);
                self.open(5, "TimePeriod");
                self.tag(6, "FromDate", format!("{}T00:00:00", e.from));
                self.tag(6, "ToDate", format!("{}T23:59:00", e.to));
                self.close(5, "TimePeriod");
                self.tag(5, "Occurrences", 1);
                self.opt_s(5, "Name", &e.name);
                self.tag(5, "Type", 1);
                self.tag(5, "DayWorking", e.working as i32);
                if e.working {
                    self.working_times(5, &e.ranges);
                }
                self.close(4, "Exception");
            }
            self.close(3, "Exceptions");
        }
        self.close(2, "Calendar");
    }
}

/// Serialize a plan as MSPDI XML.
pub fn write(p: &Plan, opts: &WriteOptions) -> String {
    let mut w = W { o: String::new() };
    w.o.push_str(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Project xmlns=\"http://schemas.microsoft.com/project\">\n",
    );
    w.tag(1, "SaveVersion", 14);
    let name = p.name.clone().or(p.title.clone()).unwrap_or_else(|| "Project".into());
    w.tag(1, "Name", esc(&name));
    w.opt_s(1, "Title", &p.title);
    w.opt_s(1, "Company", &p.company);
    w.opt_s(1, "Manager", &p.manager);
    w.opt_s(1, "Author", &p.author);
    w.tag(1, "ScheduleFromStart", 1);
    let start = p.start.or_else(|| p.tasks.iter().filter_map(|t| t.start).min());
    let finish = p.finish.or_else(|| p.tasks.iter().filter_map(|t| t.finish).max());
    w.opt_dt(1, "StartDate", &start);
    w.opt_dt(1, "FinishDate", &finish);

    let calendars: Vec<CalendarRec> = if p.calendars.is_empty() { vec![CalendarRec::standard(1)] } else { p.calendars.clone() };
    let known: HashSet<i32> = calendars.iter().map(|c| c.uid).collect();
    let default_cal = p
        .default_calendar_uid
        .filter(|u| known.contains(u))
        .or_else(|| calendars.iter().find(|c| c.base_uid.is_none()).map(|c| c.uid))
        .unwrap_or(calendars[0].uid);
    w.tag(1, "CalendarUID", default_cal);
    w.tag(1, "DefaultStartTime", "08:00:00");
    w.tag(1, "DefaultFinishTime", "17:00:00");
    w.tag(1, "MinutesPerDay", p.minutes_per_day);
    w.tag(1, "MinutesPerWeek", p.minutes_per_week);
    w.tag(1, "DaysPerMonth", 20);
    w.tag(1, "DurationFormat", 7);
    w.tag(1, "WorkFormat", 2);
    w.opt_dt(1, "StatusDate", &p.status_date);
    w.tag(1, "NewTasksAreManual", 0);
    let has_activity_ids = p.tasks.iter().any(|t| t.activity_id.is_some());
    if has_activity_ids {
        w.open(1, "ExtendedAttributes");
        w.open(2, "ExtendedAttribute");
        w.tag(3, "FieldID", TEXT1_FIELD_ID);
        w.tag(3, "FieldName", "Text1");
        w.tag(3, "Alias", "Activity ID");
        w.close(2, "ExtendedAttribute");
        w.close(1, "ExtendedAttributes");
    }

    // Base calendars first so derived calendars can reference them.
    w.open(1, "Calendars");
    for c in calendars.iter().filter(|c| c.base_uid.is_none()).chain(calendars.iter().filter(|c| c.base_uid.is_some())) {
        w.calendar(c, &known);
    }
    w.close(1, "Calendars");

    w.open(1, "Tasks");
    let mut next_id = 1;
    for t in &p.tasks {
        w.open(2, "Task");
        w.tag(3, "UID", t.uid);
        let id = t.id.unwrap_or(if t.uid == 0 { 0 } else { next_id });
        next_id = next_id.max(id + 1);
        w.tag(3, "ID", id);
        if is_null_task(t) {
            // A blank row in Project's task sheet.
            w.tag(3, "IsNull", 1);
            w.tag(3, "OutlineLevel", t.outline_level.max(1));
            w.close(2, "Task");
            continue;
        }
        w.tag(3, "Name", esc(&t.name));
        w.tag(3, "Active", 1);
        w.tag(3, "Manual", t.manual as i32);
        w.tag(3, "Type", if t.summary { 1 } else { 0 });
        w.tag(3, "IsNull", 0);
        w.opt_s(3, "WBS", &t.wbs);
        w.tag(3, "OutlineLevel", t.outline_level.max(0));
        w.tag(3, "Priority", 500);
        w.opt_dt(3, "Start", &t.start);
        w.opt_dt(3, "Finish", &t.finish);
        w.tag(3, "Duration", pt(t.duration_hours));
        if t.manual {
            w.opt_dt(3, "ManualStart", &t.start);
            w.opt_dt(3, "ManualFinish", &t.finish);
            w.tag(3, "ManualDuration", pt(t.duration_hours));
        }
        w.tag(3, "DurationFormat", 7);
        w.tag(3, "Milestone", t.milestone as i32);
        w.tag(3, "Summary", t.summary as i32);
        w.tag(3, "Critical", t.critical as i32);
        // Slack is in tenths of a minute, like LinkLag.
        if let Some(h) = t.free_slack_hours {
            w.tag(3, "FreeSlack", (h * 600.0).round() as i64);
        }
        if let Some(h) = t.total_slack_hours {
            w.tag(3, "TotalSlack", (h * 600.0).round() as i64);
        }
        if let Some(c) = t.fixed_cost.filter(|c| *c != 0.0) {
            w.tag(3, "FixedCost", money(c));
        }
        w.tag(3, "PercentComplete", t.percent_complete.round() as i64);
        if let Some(c) = t.cost.filter(|c| *c != 0.0) {
            w.tag(3, "Cost", money(c));
        }
        w.opt_dt(3, "ActualStart", &t.actual_start);
        w.opt_dt(3, "ActualFinish", &t.actual_finish);
        if let Some(c) = t.actual_cost.filter(|c| *c != 0.0) {
            w.tag(3, "ActualCost", money(c));
        }
        let (mut ct, mut cd) = (t.constraint_type.unwrap_or(0), t.constraint_date);
        if opts.pin_dates
            && ct == 0
            && !t.summary
            && !t.manual
            && t.predecessors.is_empty()
            && matches!((t.start, start), (Some(ts), Some(ps)) if ts > ps)
        {
            ct = 4;
            cd = t.start;
        }
        if ct >= 2 && cd.is_none() {
            ct = 0; // a dated constraint without a date is invalid
        }
        w.tag(3, "ConstraintType", ct);
        w.tag(3, "CalendarUID", t.calendar_uid.filter(|u| known.contains(u)).unwrap_or(-1));
        if ct >= 2 {
            w.opt_dt(3, "ConstraintDate", &cd);
        }
        w.opt_dt(3, "Deadline", &t.deadline);
        w.opt_s(3, "Notes", &t.notes);
        for pr in &t.predecessors {
            w.open(3, "PredecessorLink");
            w.tag(4, "PredecessorUID", pr.predecessor_uid);
            w.tag(4, "Type", link_type_code(&pr.link_type));
            w.tag(4, "CrossProject", 0);
            w.tag(4, "LinkLag", (pr.lag_hours * 600.0).round() as i64); // tenths of a minute
            w.tag(4, "LagFormat", 7);
            w.close(3, "PredecessorLink");
        }
        if let Some(code) = t.activity_id.as_deref().filter(|_| has_activity_ids) {
            w.open(3, "ExtendedAttribute");
            w.tag(4, "FieldID", TEXT1_FIELD_ID);
            w.tag(4, "Value", esc(code));
            w.close(3, "ExtendedAttribute");
        }
        if t.baseline_start.is_some() || t.baseline_finish.is_some() {
            w.open(3, "Baseline");
            w.tag(4, "Number", 0);
            w.opt_dt(4, "Start", &t.baseline_start);
            w.opt_dt(4, "Finish", &t.baseline_finish);
            if let Some(h) = t.baseline_duration_hours {
                w.tag(4, "Duration", pt(h));
                w.tag(4, "DurationFormat", 7);
            }
            if let Some(c) = t.baseline_cost {
                w.tag(4, "Cost", money(c));
            }
            w.close(3, "Baseline");
        }
        w.close(2, "Task");
    }
    w.close(1, "Tasks");

    w.open(1, "Resources");
    for r in &p.resources {
        w.open(2, "Resource");
        w.tag(3, "UID", r.uid);
        if let Some(id) = r.id {
            w.tag(3, "ID", id);
        }
        if !r.name.is_empty() {
            w.tag(3, "Name", esc(&r.name));
        }
        w.tag(
            3,
            "Type",
            match r.rtype.as_str() {
                "Material" => 0,
                "Cost" => 2,
                _ => 1,
            },
        );
        w.tag(3, "IsNull", 0);
        w.opt_s(3, "Initials", &r.initials);
        w.opt_s(3, "Group", &r.group);
        w.opt_s(3, "EmailAddress", &r.email);
        if r.rtype == "Work" {
            w.tag(3, "MaxUnits", format!("{:.2}", r.max_units.unwrap_or(100.0) / 100.0));
        }
        if let Some(v) = r.standard_rate {
            w.tag(3, "StandardRate", v);
            w.tag(3, "StandardRateFormat", 2);
        }
        if let Some(c) = r.cost.filter(|c| *c != 0.0) {
            w.tag(3, "Cost", money(c));
        }
        if let Some(c) = r.cost_per_use.filter(|c| *c != 0.0) {
            w.tag(3, "CostPerUse", money(c));
        }
        w.tag(3, "CalendarUID", r.calendar_uid.filter(|u| known.contains(u)).unwrap_or(default_cal));
        w.opt_s(3, "Notes", &r.notes);
        w.tag(3, "IsCostResource", (r.rtype == "Cost") as i32);
        w.close(2, "Resource");
    }
    w.close(1, "Resources");

    w.open(1, "Assignments");
    for a in &p.assignments {
        w.open(2, "Assignment");
        w.tag(3, "UID", a.uid);
        w.tag(3, "TaskUID", a.task_uid);
        w.tag(3, "ResourceUID", a.resource_uid);
        if let Some(c) = a.cost.filter(|c| *c != 0.0) {
            w.tag(3, "Cost", money(c));
        }
        w.opt_dt(3, "Finish", &a.finish);
        w.opt_dt(3, "Start", &a.start);
        w.tag(3, "Units", a.units / 100.0);
        w.tag(3, "Work", pt(a.work_hours));
        w.close(2, "Assignment");
    }
    w.close(1, "Assignments");
    w.o.push_str("</Project>\n");
    w.o
}

/// MSPDI field ID of the task Text1 custom field (used for Primavera activity IDs).
pub const TEXT1_FIELD_ID: i32 = 188743731;

/// Blank task-sheet rows ("null tasks" in MPP files) have no name and no dates.
pub fn is_null_task(t: &TaskRec) -> bool {
    t.name.is_empty() && t.start.is_none() && t.finish.is_none() && t.uid != 0
}

/// MSPDI PredecessorLink/Type code: 0=FF, 1=FS, 2=SF, 3=SS.
pub fn link_type_code(t: &str) -> i32 {
    match t {
        "FF" => 0,
        "SF" => 2,
        "SS" => 3,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xsd_durations() {
        assert_eq!(pt(8.0), "PT8H0M0S");
        assert_eq!(pt(1.5), "PT1H30M0S");
        assert_eq!(pt(-3.0), "PT0H0M0S");
    }

    #[test]
    fn escaping() {
        assert_eq!(esc("R&D <x> \"q\" 'a'\u{1}"), "R&amp;D &lt;x&gt; &quot;q&quot; &apos;a&apos;");
    }

    #[test]
    fn link_codes() {
        assert_eq!([link_type_code("FF"), link_type_code("FS"), link_type_code("SF"), link_type_code("SS")], [0, 1, 2, 3]);
    }

    #[test]
    fn writes_minimal_plan() {
        let p = Plan {
            minutes_per_day: 480,
            minutes_per_week: 2400,
            tasks: vec![TaskRec { uid: 1, id: Some(1), name: "A".into(), outline_level: 1, ..Default::default() }],
            ..Default::default()
        };
        let x = write(&p, &WriteOptions::default());
        assert!(x.contains("<Project xmlns=\"http://schemas.microsoft.com/project\">"));
        assert!(x.contains("<Name>Standard</Name>"));
        assert!(x.contains("<CalendarUID>1</CalendarUID>"));
    }
}
