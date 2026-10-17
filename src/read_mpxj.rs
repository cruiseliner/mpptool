//! Reader backed by the `mpxj-rs` crate (MPP8/9/12/14, MPX, MSPDI XML).
use crate::model::*;
use chrono::{NaiveTime, Weekday};
use mpxj::day_type::DayType;
use mpxj::duration::Duration;
use mpxj::mpx::locale_utility::MpxLocale;
use mpxj::project_calendar::ProjectCalendar;
use mpxj::project_entity_with_id::ProjectEntityWithID;
#[allow(unused_imports)]
use mpxj::project_entity_with_unique_id::ProjectEntityWithUniqueID;
use mpxj::rate::Rate;
use mpxj::reader::project_reader::ProjectReader;
use mpxj::relation_type::RelationType;
use mpxj::task_mode::TaskMode;
use mpxj::time_unit::TimeUnit;
use std::path::Path;

/// Input file formats understood by this reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mpp,
    Mpx,
    Mspdi,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Mpp => "MPP",
            Format::Mpx => "MPX",
            Format::Mspdi => "MSPDI",
        }
    }
}

/// Detect the file format from its content (not its extension).
pub fn detect_bytes(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        return Some(Format::Mpp);
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let head = head.trim_start_matches('\u{feff}');
    if head.starts_with("MPX") {
        return Some(Format::Mpx);
    }
    if head.contains("schemas.microsoft.com/project") {
        return Some(Format::Mspdi);
    }
    None
}

pub fn detect(path: &Path) -> Result<Format, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    detect_bytes(&bytes).ok_or_else(|| format!("{}: unrecognised format (expected MPP, MPX or Microsoft Project XML)", path.display()))
}

/// Parse an MPX locale code (en, de, fr, it, pt, sv, zh, es, ru).
pub fn parse_locale(s: &str) -> Result<MpxLocale, String> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "en" => MpxLocale::En,
        "de" => MpxLocale::De,
        "fr" => MpxLocale::Fr,
        "it" => MpxLocale::It,
        "pt" => MpxLocale::Pt,
        "sv" => MpxLocale::Sv,
        "zh" => MpxLocale::Zh,
        "es" => MpxLocale::Es,
        "ru" => MpxLocale::Ru,
        _ => return Err(format!("unsupported MPX locale '{s}' (use en, de, fr, it, pt, sv, zh, es, ru)")),
    })
}

/// Convert a duration to working hours using the project's minutes per day/week.
pub fn dur_hours(d: &Option<Duration>, mpd: f64, mpw: f64) -> f64 {
    let Some(d) = d else { return 0.0 };
    let v = d.get_duration();
    match d.get_units() {
        TimeUnit::Minutes | TimeUnit::ElapsedMinutes => v / 60.0,
        TimeUnit::Hours | TimeUnit::ElapsedHours => v,
        TimeUnit::Days => v * mpd / 60.0,
        TimeUnit::Weeks => v * mpw / 60.0,
        TimeUnit::Months => v * 20.0 * mpd / 60.0,
        TimeUnit::Years => v * 260.0 * mpd / 60.0,
        TimeUnit::ElapsedDays => v * 24.0,
        TimeUnit::ElapsedWeeks => v * 168.0,
        TimeUnit::ElapsedMonths => v * 720.0,
        TimeUnit::ElapsedYears => v * 8760.0,
        _ => 0.0,
    }
}

fn rate_per_hour(r: &Option<Rate>, mpd: f64, mpw: f64) -> Option<f64> {
    let r = r.as_ref()?;
    let a = r.get_amount();
    Some(match r.get_units() {
        TimeUnit::Minutes | TimeUnit::ElapsedMinutes => a * 60.0,
        TimeUnit::Days => a / (mpd / 60.0),
        TimeUnit::Weeks => a / (mpw / 60.0),
        TimeUnit::Months => a / (20.0 * mpd / 60.0),
        TimeUnit::Years => a / (260.0 * mpd / 60.0),
        _ => a,
    })
}

fn dur_display(d: &Option<Duration>) -> String {
    let Some(d) = d else { return String::new() };
    let u = match d.get_units() {
        TimeUnit::Minutes => "m",
        TimeUnit::Hours => "h",
        TimeUnit::Days => "d",
        TimeUnit::Weeks => "w",
        TimeUnit::Months => "mo",
        TimeUnit::Percent => "%",
        TimeUnit::Years => "y",
        TimeUnit::ElapsedMinutes => "em",
        TimeUnit::ElapsedHours => "eh",
        TimeUnit::ElapsedDays => "ed",
        TimeUnit::ElapsedWeeks => "ew",
        TimeUnit::ElapsedMonths => "emo",
        TimeUnit::ElapsedYears => "ey",
        _ => "",
    };
    let v = d.get_duration();
    if v.fract() == 0.0 { format!("{}{}", v as i64, u) } else { format!("{v:.2}{u}") }
}

fn non_empty(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

fn ranges<'a>(it: impl Iterator<Item = &'a mpxj::local_time_range::LocalTimeRange>) -> Vec<(NaiveTime, NaiveTime)> {
    it.filter_map(|r| Some((r.get_start()?, r.get_end()?))).collect()
}

const WEEK: [Weekday; 7] = [Weekday::Sun, Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat];

fn convert_calendar(c: &ProjectCalendar) -> Option<CalendarRec> {
    let uid = c.get_unique_id()?;
    let base_uid = c.get_parent_unique_id();
    let mut days: [DayRec; 7] = Default::default();
    for (i, wd) in WEEK.iter().enumerate() {
        let own = c.get_calendar_day_type(*wd);
        let ty = if base_uid.is_none() { c.get_day_type(*wd) } else { own };
        days[i] = match ty {
            Some(DayType::Working) => {
                let hours = if base_uid.is_none() { c.get_hours(*wd) } else { c.get_calendar_hours_for_day(*wd).cloned() };
                DayRec { working: Some(true), ranges: hours.map(|h| ranges(h.iter())).unwrap_or_default() }
            }
            Some(DayType::NonWorking) => DayRec { working: Some(false), ranges: vec![] },
            _ => DayRec { working: None, ranges: vec![] },
        };
    }
    let exceptions = c
        .get_expanded_calendar_exceptions()
        .iter()
        .filter_map(|e| {
            Some(ExceptionRec {
                name: e.get_name().map(str::to_string),
                from: e.get_from_date()?,
                to: e.get_to_date().or(e.get_from_date())?,
                working: e.get_working(),
                ranges: ranges(e.iter()),
            })
        })
        .collect();
    Some(CalendarRec { uid, name: c.get_name().unwrap_or("Calendar").to_string(), base_uid, days, exceptions })
}

/// Read options.
#[derive(Debug, Clone, Default)]
pub struct ReadOptions {
    /// Locale for MPX files (default English).
    pub mpx_locale: Option<String>,
    /// Project to read from multi-project files (Primavera): ObjectId/proj_id, short name or name.
    pub project: Option<String>,
}

pub fn read(path: &Path, opts: &ReadOptions) -> Result<Plan, String> {
    let fmt = detect(path)?;
    let res = match fmt {
        Format::Mpp => mpxj::mpp::mpp_reader::MPPReader::new().read_file(path),
        Format::Mpx => {
            let mut r = mpxj::mpx::mpx_reader::MPXReader::new();
            if let Some(l) = &opts.mpx_locale {
                r.set_locale(parse_locale(l)?);
            }
            r.read_file(path)
        }
        Format::Mspdi => mpxj::mspdi::mspdi_reader::MSPDIReader::new().read_file(path),
    };
    let pf = res.map_err(|e| {
        let hint = if fmt == Format::Mpx && opts.mpx_locale.is_none() { " (non-English MPX? try --locale de|fr|...)" } else { "" };
        format!("failed to read {} as {}: {e}{hint}", path.display(), fmt.name())
    })?;
    let pf = pf.borrow();
    // Files written by mpptool from Primavera carry the P6 activity ID in Text1 with this alias.
    let text1_is_activity_id = fmt == Format::Mspdi
        && std::fs::read(path).map(|b| String::from_utf8_lossy(&b).contains("<Alias>Activity ID</Alias>")).unwrap_or(false);
    let props = pf.get_project_properties();
    let mpd = props.get_minutes_per_day().filter(|v| *v > 0).unwrap_or(480);
    let mpw = props.get_minutes_per_week().filter(|v| *v > 0).unwrap_or(2400);
    let (mpdf, mpwf) = (mpd as f64, mpw as f64);
    let fname = fmt.name();
    let mut plan = Plan {
        format: match props.get_file_type() {
            Some(t) if t != fname => format!("{fname} ({t})"),
            _ => fname.to_string(),
        },
        title: props.get_project_title(),
        name: props.get_name(),
        author: props.get_author(),
        manager: props.get_manager(),
        company: props.get_company(),
        start: props.get_start_date().or(pf.get_earliest_start_date()),
        finish: props.get_finish_date().or(pf.get_latest_finish_date()),
        minutes_per_day: mpd,
        minutes_per_week: mpw,
        default_calendar_uid: props.get_default_calendar_unique_id(),
        status_date: props.get_status_date(),
        ..Default::default()
    };
    for c in pf.get_calendars().iter() {
        if let Ok(c) = c.try_borrow()
            && let Some(rec) = convert_calendar(&c)
        {
            plan.calendars.push(rec);
        }
    }
    for t in pf.get_tasks().iter() {
        let t = t.borrow();
        let Some(uid) = t.get_unique_id() else { continue };
        let preds = t
            .get_predecessors()
            .iter()
            .filter_map(|r| {
                let r = r.borrow();
                let p = r.get_predecessor_task()?;
                let p = p.borrow();
                Some(Pred {
                    predecessor_uid: p.get_unique_id()?,
                    predecessor_id: p.get_id(),
                    link_type: match r.get_type() {
                        RelationType::FinishFinish => "FF",
                        RelationType::StartFinish => "SF",
                        RelationType::StartStart => "SS",
                        _ => "FS",
                    }
                    .into(),
                    lag_hours: dur_hours(&r.get_lag(), mpdf, mpwf),
                    lag_display: dur_display(&r.get_lag()),
                })
            })
            .collect();
        let ct = t.get_constraint_type().map(|c| c as i32).filter(|c| (1..=7).contains(c)); // ASAP = None
        plan.tasks.push(TaskRec {
            uid,
            id: t.get_id(),
            name: t.get_name().unwrap_or_default(),
            outline_level: t.get_outline_level().unwrap_or(1),
            wbs: t.get_wbs(),
            // Any MSP custom text field aliased "Activity ID" (survives MS Project re-saves that move or keep Text1).
            activity_id: match t.get_field_by_alias("Activity ID") {
                Some(mpxj::field_value::FieldValue::String(v)) if !v.trim().is_empty() => Some(v),
                _ if text1_is_activity_id => t.get_text(1),
                _ => None,
            },
            start: t.get_start(),
            finish: t.get_finish(),
            duration_hours: dur_hours(&t.get_duration(), mpdf, mpwf),
            percent_complete: t.get_percentage_complete().unwrap_or(0.0),
            milestone: t.get_milestone(),
            summary: t.get_summary(),
            critical: t.get_critical(),
            manual: t.get_task_mode() == Some(TaskMode::ManuallyScheduled),
            predecessors: preds,
            baseline_start: t.get_baseline_start(),
            baseline_finish: t.get_baseline_finish(),
            total_slack_hours: t.get_total_slack().map(|d| dur_hours(&Some(d), mpdf, mpwf)),
            notes: non_empty(t.get_notes()),
            constraint_type: ct,
            constraint_date: if ct.unwrap_or(0) >= 2 { t.get_constraint_date() } else { None },
            deadline: t.get_deadline(),
            actual_start: t.get_actual_start(),
            actual_finish: t.get_actual_finish(),
            cost: t.get_cost(),
            fixed_cost: t.get_fixed_cost(),
            actual_cost: t.get_actual_cost(),
            calendar_uid: t.get_calendar_unique_id(),
            baseline_duration_hours: t.get_baseline_duration().map(|d| dur_hours(&Some(d), mpdf, mpwf)),
            baseline_cost: t.get_baseline_cost(),
            free_slack_hours: None,
        });
    }
    for r in pf.get_resources().iter() {
        let r = r.borrow();
        let Some(uid) = r.get_unique_id() else { continue };
        plan.resources.push(ResourceRec {
            uid,
            id: r.get_id(),
            name: r.get_name().unwrap_or_default(),
            rtype: format!("{:?}", r.get_type()),
            initials: r.get_initials(),
            max_units: r.get_max_units(),
            email: r.get_email_address(),
            group: r.get_group(),
            standard_rate: rate_per_hour(&r.get_standard_rate(), mpdf, mpwf),
            cost_per_use: r.get_cost_per_use(),
            cost: r.get_cost(),
            calendar_uid: r.get_calendar_unique_id(),
            notes: non_empty(r.get_notes()),
        });
    }
    for a in pf.get_resource_assignments().iter() {
        let a = a.borrow();
        let (Some(tu), Some(ru)) = (a.get_task_unique_id(), a.get_resource_unique_id()) else { continue };
        plan.assignments.push(AssignmentRec {
            uid: a.get_unique_id().unwrap_or(0),
            task_uid: tu,
            resource_uid: ru,
            units: a.get_units().unwrap_or(100.0),
            work_hours: dur_hours(&a.get_work(), mpdf, mpwf),
            start: a.get_start(),
            finish: a.get_finish(),
            cost: a.get_cost(),
        });
    }
    plan.resolve_pred_ids();
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_formats() {
        assert_eq!(detect_bytes(&[0xD0, 0xCF, 0x11, 0xE0, 0, 0]), Some(Format::Mpp));
        assert_eq!(detect_bytes(b"MPX,Microsoft Project,4.0"), Some(Format::Mpx));
        assert_eq!(detect_bytes(b"<?xml version=\"1.0\"?><Project xmlns=\"http://schemas.microsoft.com/project\">"), Some(Format::Mspdi));
        assert_eq!(detect_bytes(b"<APIBusinessObjects/>"), None);
    }

    #[test]
    fn locale_parsing() {
        assert!(parse_locale("DE").is_ok());
        assert!(parse_locale("xx").is_err());
    }

    #[test]
    fn duration_conversion() {
        let d = Some(Duration::get_instance(2.0, TimeUnit::Days));
        assert_eq!(dur_hours(&d, 480.0, 2400.0), 16.0);
        let w = Some(Duration::get_instance(1.0, TimeUnit::Weeks));
        assert_eq!(dur_hours(&w, 480.0, 2400.0), 40.0);
        assert_eq!(dur_hours(&None, 480.0, 2400.0), 0.0);
    }
}
