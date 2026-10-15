//! mpptool — read/analyze Microsoft Project files (MPP/MPX/MSPDI XML) and export MSPDI XML.
use clap::{Parser, Subcommand, ValueEnum};
use mpptool::model::*;
use mpptool::mspdi_writer::{self, WriteOptions};
use mpptool::{Engine, ReadOptions, from_csv};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "mpptool", version, about = "Read/analyze Microsoft Project files and export Microsoft Project XML (MSPDI)")]
struct Cli {
    /// Reader engine
    #[arg(long, global = true, value_enum, default_value_t = EngineArg::Mpxj)]
    engine: EngineArg,
    /// Locale of MPX files (en, de, fr, it, pt, sv, zh, es, ru)
    #[arg(long, global = true)]
    locale: Option<String>,
    /// Project to read from multi-project Primavera files (id, short name or name); default: the first
    #[arg(long, global = true)]
    project: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Copy, Clone, ValueEnum)]
enum EngineArg {
    /// mpxj-rs: MPP8/9/12/14, MPX, MSPDI XML
    Mpxj,
    /// mpxrust: MPP14 only, experimental
    Mpxrust,
}

#[derive(Subcommand)]
enum Cmd {
    /// Project properties, counts, dates, cost, critical-path info
    Summary {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// List tasks (table, --json or --csv)
    Tasks {
        file: PathBuf,
        #[arg(long, conflicts_with = "csv")]
        json: bool,
        #[arg(long)]
        csv: bool,
    },
    /// List resources (table, --json or --csv)
    Resources {
        file: PathBuf,
        #[arg(long, conflicts_with = "csv")]
        json: bool,
        #[arg(long)]
        csv: bool,
    },
    /// List resource assignments (table or --json)
    Assignments {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// List the projects contained in a file (Primavera files can hold several)
    Projects { file: PathBuf },
    /// List calendars and their exceptions (table or --json)
    Calendars {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Convert a readable file (.mpp/.mpx/.xml) to Microsoft Project XML (MSPDI)
    ToXml {
        file: PathBuf,
        out: PathBuf,
        /// Pin dates of unlinked, unconstrained tasks with Start-No-Earlier-Than constraints
        #[arg(long)]
        pin_dates: bool,
    },
    /// Schedule a new plan from CSV (name,start,duration_days,predecessors,outline_level[,resources,notes]) -> MSPDI XML
    FromCsv { input: PathBuf, out: PathBuf },
    /// Render a Gantt chart: self-contained offline HTML (.html) or plain SVG (.svg)
    Gantt {
        file: PathBuf,
        /// Output file; .svg writes plain SVG, anything else an HTML page
        out: PathBuf,
        /// Only show critical tasks (and their summary rows)
        #[arg(long)]
        critical_only: bool,
        /// Window start (YYYY-MM-DD)
        #[arg(long, value_parser = parse_date)]
        from: Option<chrono::NaiveDate>,
        /// Window end (YYYY-MM-DD)
        #[arg(long, value_parser = parse_date)]
        to: Option<chrono::NaiveDate>,
        /// Chart title (default: project title)
        #[arg(long)]
        title: Option<String>,
        /// Date for the "today" line (default: the current date)
        #[arg(long, value_parser = parse_date)]
        today: Option<chrono::NaiveDate>,
    },
    /// DCMA 14-point schedule assessment (table or --json)
    Check {
        file: PathBuf,
        #[arg(long)]
        json: bool,
        /// Status date to assess against (YYYY-MM-DD); default: the file's status/data date
        #[arg(long, value_parser = parse_date)]
        status_date: Option<chrono::NaiveDate>,
        /// Exit with status 1 when any check fails
        #[arg(long)]
        fail_on_error: bool,
    },
}

fn parse_date(s: &str) -> Result<chrono::NaiveDate, String> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| format!("'{s}': {e} (want YYYY-MM-DD)"))
}

fn main() -> ExitCode {
    // Behave like a normal Unix tool when piped into `head` etc.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn json<T: serde::Serialize>(v: &T) -> Result<(), String> {
    let s = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    println!("{s}");
    Ok(())
}

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    std::fs::write(path, content).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn run(cli: Cli) -> Result<(), String> {
    let engine = match cli.engine {
        EngineArg::Mpxj => Engine::Mpxj,
        EngineArg::Mpxrust => Engine::Mpxrust,
    };
    let opts = ReadOptions { mpx_locale: cli.locale.clone(), project: cli.project.clone() };
    let read = |f: &Path| mpptool::read(f, &opts, engine);
    match cli.cmd {
        Cmd::Summary { file, json: j } => {
            let s = read(&file)?.summary();
            if j { json(&s) } else { print_summary(&s) }
        }
        Cmd::Tasks { file, json: j, csv } => {
            let p = read(&file)?;
            if j {
                json(&p.tasks)
            } else if csv {
                write_tasks_csv(&p)
            } else {
                print_tasks(&p)
            }
        }
        Cmd::Resources { file, json: j, csv } => {
            let p = read(&file)?;
            if j {
                json(&p.resources)
            } else if csv {
                let mut w = csv::Writer::from_writer(std::io::stdout());
                for r in &p.resources {
                    w.serialize(r).map_err(|e| e.to_string())?;
                }
                w.flush().map_err(|e| e.to_string())
            } else {
                println!(
                    "{:>5} {:>5} {:<35} {:<9} {:<8} {:>8} {:>10} Email/Group",
                    "UID", "ID", "Name", "Type", "Initials", "MaxUnit", "Rate/h"
                );
                for r in &p.resources {
                    println!(
                        "{:>5} {:>5} {:<35} {:<9} {:<8} {:>8} {:>10} {} {}",
                        r.uid,
                        opt_i(r.id),
                        trunc(&r.name, 35),
                        r.rtype,
                        r.initials.clone().unwrap_or_default(),
                        r.max_units.map(|u| format!("{u:.0}%")).unwrap_or_default(),
                        r.standard_rate.filter(|v| *v != 0.0).map(|v| format!("{v:.2}")).unwrap_or_default(),
                        r.email.clone().unwrap_or_default(),
                        r.group.clone().unwrap_or_default()
                    );
                }
                Ok(())
            }
        }
        Cmd::Assignments { file, json: j } => {
            let p = read(&file)?;
            if j {
                return json(&p.assignments);
            }
            println!("{:>5} {:>7} {:>7} {:>7} {:>9} {:>10}", "UID", "TaskUID", "ResUID", "Units%", "Work(h)", "Cost");
            for a in &p.assignments {
                println!(
                    "{:>5} {:>7} {:>7} {:>7.0} {:>9.2} {:>10}",
                    a.uid,
                    a.task_uid,
                    a.resource_uid,
                    a.units,
                    a.work_hours,
                    a.cost.map(|c| format!("{c:.2}")).unwrap_or_default()
                );
            }
            Ok(())
        }
        Cmd::Projects { file } => {
            println!("{:<12} {:<20} Name", "Id", "Short name");
            for (id, short, name) in mpptool::list_projects(&file, &opts, engine)? {
                println!("{id:<12} {short:<20} {name}");
            }
            Ok(())
        }
        Cmd::Calendars { file, json: j } => {
            let p = read(&file)?;
            if j {
                return json(&p.calendars);
            }
            const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
            for c in &p.calendars {
                let def = if Some(c.uid) == p.default_calendar_uid { " (project default)" } else { "" };
                println!("[{}] {}{}{}", c.uid, c.name, c.base_uid.map(|b| format!(" — based on {b}")).unwrap_or_default(), def);
                for (i, d) in c.days.iter().enumerate() {
                    if let Some(w) = d.working {
                        let r: Vec<String> = d.ranges.iter().map(|(a, b)| format!("{}-{}", a.format("%H:%M"), b.format("%H:%M"))).collect();
                        println!("    {}: {}", DAYS[i], if w { r.join(", ") } else { "non-working".into() });
                    }
                }
                for e in &c.exceptions {
                    let span = if e.from == e.to { e.from.to_string() } else { format!("{} – {}", e.from, e.to) };
                    println!(
                        "    exception {span}: {}{}",
                        if e.working { "working" } else { "non-working" },
                        e.name.as_ref().map(|n| format!(" ({n})")).unwrap_or_default()
                    );
                }
            }
            Ok(())
        }
        Cmd::ToXml { file, out, pin_dates } => {
            let p = read(&file)?;
            write_file(&out, &mspdi_writer::write(&p, &WriteOptions { pin_dates }))?;
            eprintln!(
                "wrote {} ({} tasks, {} resources, {} assignments, {} calendars)",
                out.display(),
                p.tasks.len(),
                p.resources.len(),
                p.assignments.len(),
                p.calendars.len()
            );
            Ok(())
        }
        Cmd::FromCsv { input, out } => {
            let p = from_csv::build(&input)?;
            write_file(&out, &mspdi_writer::write(&p, &WriteOptions::default()))?;
            eprintln!(
                "wrote {} ({} tasks, finish {}, {} critical)",
                out.display(),
                p.tasks.len(),
                fmt_dt(&p.finish),
                p.tasks.iter().filter(|t| t.critical && !t.summary).count()
            );
            Ok(())
        }
        Cmd::Gantt { file, out, critical_only, from, to, title, today } => {
            let p = read(&file)?;
            let o = mpptool::gantt::GanttOptions { critical_only, from, to, title, today };
            let svg = out.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"));
            let doc = if svg { mpptool::gantt::render_svg(&p, &o) } else { mpptool::gantt::render_html(&p, &o) };
            write_file(&out, &doc)?;
            eprintln!("wrote {} ({} tasks)", out.display(), p.tasks.len());
            Ok(())
        }
        Cmd::Check { file, json: j, status_date, fail_on_error } => {
            let p = read(&file)?;
            let o = mpptool::check::CheckOptions { status_date: status_date.and_then(|d| d.and_hms_opt(17, 0, 0)) };
            let a = mpptool::check::assess(&p, &o);
            if j {
                json(&a)?;
            } else {
                print_check(&a);
            }
            if fail_on_error && a.failed > 0 {
                return Err(format!("{} of 14 checks failed", a.failed));
            }
            Ok(())
        }
    }
}

fn print_check(a: &mpptool::check::Assessment) {
    println!(
        "DCMA 14-point assessment: {} activities ({} incomplete), {} relationships, status date {}",
        a.activities,
        a.incomplete_activities,
        a.relationships,
        a.status_date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_else(|| "none".into())
    );
    println!("{:>2}  {:<44} {:<8} {:>10} {:>8} {:>8}  Offending IDs / note", "#", "Check", "Result", "Threshold", "Count", "Value");
    for c in &a.checks {
        let value = match (c.value, c.percent) {
            (Some(v), _) => format!("{v:.2}"),
            (None, Some(p)) => format!("{p:.1}%"),
            _ => String::new(),
        };
        let mut ids = c.task_ids.iter().take(10).cloned().collect::<Vec<_>>().join(",");
        if c.task_ids.len() > 10 {
            ids += &format!(",… (+{})", c.task_ids.len() - 10);
        }
        let extra = if ids.is_empty() { c.note.clone().unwrap_or_default() } else { ids };
        let count = if c.status == "skipped" { String::new() } else { format!("{}/{}", c.count, c.total) };
        println!(
            "{:>2}  {:<44} {:<8} {:>10} {:>8} {:>8}  {}",
            c.number,
            trunc(&c.name, 44),
            c.status.to_uppercase(),
            c.threshold,
            count,
            value,
            extra
        );
    }
    println!("passed {}, failed {}, skipped {}", a.passed, a.failed, a.skipped);
}

fn opt_i(v: Option<i32>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() > n { s.chars().take(n - 1).collect::<String>() + "…" } else { s.to_string() }
}

fn print_tasks(p: &Plan) -> Result<(), String> {
    let mut out = std::io::stdout().lock();
    let hpd = p.minutes_per_day as f64 / 60.0;
    let _ = writeln!(
        out,
        "{:>5} {:>5} {:<3} {:<45} {:<16} {:<16} {:>8} {:>4} {:<2} Predecessors (IDs)",
        "ID", "UID", "LVL", "Name", "Start", "Finish", "Dur(d)", "%", "F"
    );
    for t in &p.tasks {
        let name = format!("{}{}", "  ".repeat((t.outline_level.max(1) - 1) as usize), t.name);
        let flags = format!("{}{}", if t.milestone { "M" } else { "" }, if t.critical && !t.summary { "C" } else { "" });
        let _ = writeln!(
            out,
            "{:>5} {:>5} {:<3} {:<45} {:<16} {:<16} {:>8.2} {:>4.0} {:<2} {}",
            opt_i(t.id),
            t.uid,
            t.outline_level,
            trunc(&name, 45),
            fmt_dt(&t.start),
            fmt_dt(&t.finish),
            t.duration_hours / hpd,
            t.percent_complete,
            flags,
            preds_str(&t.predecessors, true)
        );
    }
    Ok(())
}

fn print_summary(s: &Summary) -> Result<(), String> {
    println!("File format:      {}", s.format);
    println!("Title:            {}", s.title.clone().unwrap_or_default());
    for (k, v) in [("Name", &s.name), ("Author", &s.author), ("Manager", &s.manager), ("Company", &s.company)] {
        if let Some(v) = v {
            println!("{:<17} {}", format!("{k}:"), v);
        }
    }
    println!("Start:            {}", fmt_dt(&s.start));
    println!("Finish:           {}", fmt_dt(&s.finish));
    println!(
        "Tasks:            {} ({} summary, {} milestones, {} complete, {} critical)",
        s.task_count, s.summary_tasks, s.milestones, s.completed_tasks, s.critical_tasks
    );
    println!("Resources:        {}", s.resource_count);
    println!("Assignments:      {}", s.assignment_count);
    println!("Dependencies:     {}", s.relation_count);
    println!("Calendars:        {} ({} exceptions)", s.calendar_count, s.calendar_exception_count);
    println!("Tasks w/baseline: {}", s.tasks_with_baseline);
    if let Some(c) = s.total_cost {
        println!("Total cost:       {c:.2}");
    }
    println!("Overall % compl.: {:.1}", s.overall_percent_complete);
    if !s.critical_path.is_empty() {
        println!("Critical (non-summary) tasks:");
        for c in &s.critical_path {
            println!("  - {c}");
        }
    }
    Ok(())
}

fn write_tasks_csv(p: &Plan) -> Result<(), String> {
    let mut w = csv::Writer::from_writer(std::io::stdout());
    let e = |e: csv::Error| e.to_string();
    w.write_record([
        "id",
        "uid",
        "name",
        "outline_level",
        "start",
        "finish",
        "duration_hours",
        "duration_days",
        "percent_complete",
        "milestone",
        "summary",
        "critical",
        "predecessors",
        "predecessor_uids",
        "total_slack_hours",
        "constraint",
        "constraint_date",
        "deadline",
        "actual_start",
        "actual_finish",
        "cost",
        "baseline_start",
        "baseline_finish",
        "notes",
        "activity_id",
    ])
    .map_err(e)?;
    let hpd = p.minutes_per_day as f64 / 60.0;
    let f = |v: Option<f64>| v.map(|v| format!("{v:.2}")).unwrap_or_default();
    for t in &p.tasks {
        w.write_record([
            opt_i(t.id),
            t.uid.to_string(),
            t.name.clone(),
            t.outline_level.to_string(),
            fmt_dt(&t.start),
            fmt_dt(&t.finish),
            format!("{:.2}", t.duration_hours),
            format!("{:.2}", t.duration_hours / hpd),
            format!("{}", t.percent_complete),
            t.milestone.to_string(),
            t.summary.to_string(),
            t.critical.to_string(),
            preds_str(&t.predecessors, true),
            preds_str(&t.predecessors, false),
            f(t.total_slack_hours),
            t.constraint_type.and_then(|c| CONSTRAINT_NAMES.get(c as usize)).unwrap_or(&"").to_string(),
            fmt_dt(&t.constraint_date),
            fmt_dt(&t.deadline),
            fmt_dt(&t.actual_start),
            fmt_dt(&t.actual_finish),
            f(t.cost),
            fmt_dt(&t.baseline_start),
            fmt_dt(&t.baseline_finish),
            t.notes.clone().unwrap_or_default(),
            t.activity_id.clone().unwrap_or_default(),
        ])
        .map_err(e)?;
    }
    w.flush().map_err(|e| e.to_string())
}
