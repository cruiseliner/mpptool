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
    /// Reader engine (only `mpxj`; kept for compatibility)
    #[arg(long, global = true, value_enum, default_value_t = EngineArg::Mpxj, hide_possible_values = true)]
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
    /// Removed in 0.6.1 (accepted only to print a clear error)
    #[value(hide = true)]
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
    FromCsv {
        input: PathBuf,
        out: PathBuf,
        #[command(flatten)]
        cal: CalArgs,
    },
    /// Monte Carlo schedule risk for any schedule (.csv plan, .json task list, .mpp/.mpx/.xml/.xer/P6 XML):
    /// P10/P50/P80/P90 finish and criticality index (calendar-aware)
    Risk {
        input: PathBuf,
        /// Number of simulation runs
        #[arg(long, default_value_t = 10_000)]
        runs: usize,
        /// Random seed (same seed, same result)
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Optimistic duration as a fraction of duration_days, when the CSV has no `optimistic` column value
        #[arg(long = "opt", default_value_t = 0.8)]
        opt_factor: f64,
        /// Pessimistic duration as a multiple of duration_days, when the CSV has no `pessimistic` column value
        #[arg(long = "pess", default_value_t = 1.5)]
        pess_factor: f64,
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        cal: CalArgs,
    },
    /// Critical path of any schedule (.csv, .json, .mpp/.mpx/.xml/.xer/P6 XML): forward/backward pass, total and free float
    Cpm {
        input: PathBuf,
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        cal: CalArgs,
    },
    /// List public holidays of built-in countries (table, --csv or --json)
    Holidays {
        /// Country codes, comma-separated (US, CA, GB, AU, DE, FR, IN, MX); `list` shows the supported countries
        #[arg(long, default_value = "US")]
        country: String,
        /// Year or range, e.g. 2026 or 2026-2028 (default: the current year)
        #[arg(long)]
        years: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, conflicts_with = "json")]
        csv: bool,
    },
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
    /// Compare two versions of a plan: tasks added/removed, changed dates, durations, links, % complete
    Compare {
        old: PathBuf,
        new: PathBuf,
        /// Output format
        #[arg(long, value_enum, default_value_t = ReportFormat::Text)]
        format: ReportFormat,
        /// Write the report to a file instead of stdout
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// How to match tasks between the two files
        #[arg(long = "match", value_enum, default_value_t = MatchArg::Auto)]
        match_by: MatchArg,
    },
    /// Baseline vs current/actual slip per task, with earned value (PV, EV, AC, SPI, CPI, SV, CV) when costs exist
    Variance {
        file: PathBuf,
        #[arg(long, conflicts_with = "csv")]
        json: bool,
        #[arg(long)]
        csv: bool,
        /// Status date for earned value (YYYY-MM-DD); default: the file's status/data date
        #[arg(long, value_parser = parse_date)]
        status_date: Option<chrono::NaiveDate>,
    },
    /// Weekly work hours per resource with over-allocation flagged: HTML heat map (.html) or CSV (.csv)
    ResourcesLoad { file: PathBuf, out: PathBuf },
    /// Export to an Excel workbook (Smartsheet/Excel friendly: indentation, predecessors by row number)
    ToXlsx { file: PathBuf, out: PathBuf },
    /// Kanban board by status (Not started / In progress / Done) with RAG flags: HTML (or --json)
    Board {
        file: PathBuf,
        /// Output .html file (omit with --json)
        out: Option<PathBuf>,
        /// Print the cards as JSON instead
        #[arg(long)]
        json: bool,
        /// Date to judge RAG against (YYYY-MM-DD); default: the file's status date, else today
        #[arg(long, value_parser = parse_date)]
        status_date: Option<chrono::NaiveDate>,
        /// Board title (default: project title)
        #[arg(long)]
        title: Option<String>,
    },
    /// Roll up every readable plan in a folder into one dashboard: HTML (.html) or JSON (.json)
    Portfolio { folder: PathBuf, out: PathBuf },
}

#[derive(Copy, Clone, ValueEnum)]
enum ReportFormat {
    Text,
    Json,
    Html,
}

#[derive(Copy, Clone, ValueEnum)]
enum MatchArg {
    /// Activity ID when both files have them, else unique ID
    Auto,
    Uid,
    ActivityId,
    Name,
}

/// Working-calendar options shared by `from-csv` and `risk`.
#[derive(clap::Args, Debug)]
struct CalArgs {
    /// Public holidays to skip: country codes, comma-separated (US = US federal; CA, GB, AU, DE, FR, IN, MX), or none
    #[arg(long, default_value = "none")]
    holidays: String,
    /// Years whose holidays apply, e.g. 2026-2028 (default: every year the schedule spans)
    #[arg(long)]
    years: Option<String>,
    /// Custom holidays / exception days: CSV lines `date[..date],name[,hours]` (hours: off, work, or 07:00-15:00;…)
    #[arg(long)]
    holiday_file: Option<PathBuf>,
    /// Working weekdays, e.g. mon-fri (default), mon-sat, all, mon,tue,thu
    #[arg(long, default_value = "mon-fri")]
    work_days: String,
    /// Daily working hours, e.g. 08:00-12:00,13:00-17:00 (default) or 06:00-16:30 for a 10-hour shift
    #[arg(long, default_value = "08:00-12:00,13:00-17:00")]
    hours: String,
    /// Override days in the weekly pattern (repeatable), e.g. sat=07:00-12:00 (Saturday overtime) or fri=off
    #[arg(long = "day", value_name = "DAYS=HOURS")]
    days: Vec<String>,
    /// Working hours in one duration day (CSV duration_days and d lags)
    #[arg(long, default_value_t = 8.0)]
    hours_per_day: f64,
    /// Calendar name written to the XML (default: Standard, or "Project Calendar" when customised)
    #[arg(long)]
    calendar_name: Option<String>,
}

impl CalArgs {
    fn spec(&self) -> Result<mpptool::calendar::CalendarSpec, String> {
        use mpptool::calendar::*;
        let hours = parse_ranges(&self.hours).map_err(|e| format!("--hours: {e}"))?;
        if hours.is_empty() {
            return Err("--hours: no working time".into());
        }
        let mut s = CalendarSpec::with_week(parse_days(&self.work_days).map_err(|e| format!("--work-days: {e}"))?, &hours);
        for d in &self.days {
            let (set, r) = parse_day_override(d).map_err(|e| format!("--day: {e}"))?;
            for (i, on) in set.iter().enumerate() {
                if *on {
                    s.week[i] = r.clone();
                }
            }
        }
        s.holidays = if self.holidays.eq_ignore_ascii_case("none") { String::new() } else { self.holidays.clone() };
        s.years = self.years.as_deref().map(mpptool::holidays::parse_years).transpose().map_err(|e| format!("--years: {e}"))?;
        if let Some(f) = &self.holiday_file {
            let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
            s.custom = parse_holiday_file(&text, &s.default_hours()).map_err(|e| format!("{}: {e}", f.display()))?;
        }
        s.hours_per_day = self.hours_per_day;
        if !s.is_standard() {
            s.name = self.calendar_name.clone().unwrap_or_else(|| "Project Calendar".into());
        } else if let Some(n) = &self.calendar_name {
            s.name = n.clone();
        }
        Ok(s)
    }
}

/// Rows + calendar for `risk`/`cpm`. Imported schedules use their own project calendar; `--holidays` adds to it.
fn load_input(
    input: &Path,
    cal: &CalArgs,
    ropts: &ReadOptions,
    engine: Engine,
) -> Result<(Vec<mpptool::from_csv::Row>, mpptool::calendar::CalendarSpec), String> {
    let csv_spec = cal.spec()?;
    let (rows, mut spec) = mpptool::risk::load_any(input, &csv_spec, ropts, engine)?;
    if !input.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv")) {
        if !csv_spec.holidays.is_empty() {
            spec.holidays = csv_spec.holidays.clone();
            spec.years = csv_spec.years;
        }
        spec.custom.extend(csv_spec.custom.iter().cloned());
    }
    Ok((rows, spec))
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
        EngineArg::Mpxrust => {
            return Err(
                "the mpxrust reader was removed in mpptool 0.6.1; use the default reader (omit --engine or pass --engine mpxj)".into()
            );
        }
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
        Cmd::FromCsv { input, out, cal } => {
            let spec = cal.spec()?;
            let p = from_csv::build_with(&input, &spec)?;
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
        Cmd::Risk { input, runs, seed, opt_factor, pess_factor, json, cal } => {
            let (rows, spec) = load_input(&input, &cal, &opts, engine)?;
            let opts = mpptool::risk::RiskOptions { runs, seed, opt_factor, pess_factor };
            let r = mpptool::risk::simulate(&rows, chrono::Local::now().date_naive(), &spec, &opts)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?);
            } else {
                print!("{}", mpptool::risk::render_text(&r));
            }
            Ok(())
        }
        Cmd::Cpm { input, json, cal } => {
            let (rows, spec) = load_input(&input, &cal, &opts, engine)?;
            let v = mpptool::risk::cpm(&rows, chrono::Local::now().date_naive(), &spec)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?);
            } else {
                println!(
                    "{:>5} {:<44} {:<16} {:<16} {:>8} {:>8} {:>8}  C",
                    "Row", "Name", "Early start", "Early finish", "Rem(d)", "TF(d)", "FF(d)"
                );
                for r in &v {
                    let name: String =
                        format!("{}{}", "  ".repeat((r.outline_level - 1).max(0) as usize), r.name).chars().take(44).collect();
                    println!(
                        "{:>5} {:<44} {:<16} {:<16} {:>8.2} {:>8.2} {:>8.2}  {}",
                        r.row,
                        name,
                        fmt_dt(&r.early_start),
                        fmt_dt(&r.early_finish),
                        r.remaining_days,
                        r.total_float_days,
                        r.free_float_days,
                        if r.critical { "*" } else { "" }
                    );
                }
            }
            Ok(())
        }
        Cmd::Holidays { country, years, json, csv } => {
            if country.eq_ignore_ascii_case("list") {
                for (c, d) in mpptool::holidays::COUNTRIES {
                    println!("{c}  {d}");
                }
                return Ok(());
            }
            let (a, b) = match years {
                Some(y) => mpptool::holidays::parse_years(&y)?,
                None => {
                    let y = chrono::Datelike::year(&chrono::Local::now().date_naive());
                    (y, y)
                }
            };
            let v = mpptool::holidays::for_years(&country, a, b)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?);
            } else if csv {
                println!("date,weekday,country,name");
                for h in &v {
                    println!("{},{},{},\"{}\"", h.date, h.date.format("%a"), h.country, h.name.replace('"', "\"\""));
                }
            } else {
                for h in &v {
                    println!("{}  {}  {}  {}", h.date, h.date.format("%a"), h.country, h.name);
                }
            }
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
        Cmd::Compare { old, new, format, out, match_by } => {
            let (a, b) = (read(&old)?, read(&new)?);
            let by = match match_by {
                MatchArg::Auto => mpptool::compare::MatchBy::Auto,
                MatchArg::Uid => mpptool::compare::MatchBy::Uid,
                MatchArg::ActivityId => mpptool::compare::MatchBy::ActivityId,
                MatchArg::Name => mpptool::compare::MatchBy::Name,
            };
            let c = mpptool::compare::compare(&a, &b, by);
            let doc = match format {
                ReportFormat::Text => mpptool::compare::to_text(&c),
                ReportFormat::Json => serde_json::to_string_pretty(&c).map_err(|e| e.to_string())? + "\n",
                ReportFormat::Html => mpptool::compare::to_html(&c, &old.display().to_string(), &new.display().to_string()),
            };
            match out {
                Some(o) => {
                    write_file(&o, &doc)?;
                    eprintln!("wrote {} ({} added, {} removed, {} changed)", o.display(), c.added.len(), c.removed.len(), c.changed.len());
                }
                None => print!("{doc}"),
            }
            Ok(())
        }
        Cmd::Variance { file, json: j, csv, status_date } => {
            let p = read(&file)?;
            let v = mpptool::variance::variance(&p, status_date.and_then(|d| d.and_hms_opt(17, 0, 0)));
            if j {
                json(&v)
            } else if csv {
                print_variance_csv(&v)
            } else {
                print_variance(&v);
                Ok(())
            }
        }
        Cmd::ResourcesLoad { file, out } => {
            let p = read(&file)?;
            let r = mpptool::resload::load(&p);
            let csv = out.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv"));
            let title = p.title.clone().or(p.name.clone()).unwrap_or_else(|| "Project".into());
            write_file(&out, &if csv { mpptool::resload::to_csv(&r)? } else { mpptool::resload::to_html(&r, &title) })?;
            let over: Vec<&str> = r.resources.iter().filter(|x| x.over_allocated_weeks > 0).map(|x| x.name.as_str()).collect();
            eprintln!(
                "wrote {} ({} resources, {} weeks, {} over-allocated{})",
                out.display(),
                r.resources.len(),
                r.weeks.len(),
                over.len(),
                if over.is_empty() { String::new() } else { format!(": {}", over.join(", ")) }
            );
            Ok(())
        }
        Cmd::ToXlsx { file, out } => {
            let p = read(&file)?;
            let n = mpptool::xlsx::write(&p, &out)?;
            eprintln!("wrote {} ({n} rows, {} resources)", out.display(), p.resources.len());
            Ok(())
        }
        Cmd::Board { file, out, json: j, status_date, title } => {
            let p = read(&file)?;
            let at =
                status_date.and_then(|d| d.and_hms_opt(17, 0, 0)).or(p.status_date).unwrap_or_else(|| chrono::Local::now().naive_local());
            let cards = mpptool::board::cards(&p, at);
            if j {
                return json(&cards);
            }
            let out = out.ok_or("board needs an output .html file (or --json)")?;
            write_file(&out, &mpptool::board::to_html(&p, &cards, at, title.as_deref()))?;
            let n = |r| cards.iter().filter(|c| c.rag == r).count();
            use mpptool::board::Rag;
            eprintln!(
                "wrote {} ({} cards: {} red, {} amber, {} green)",
                out.display(),
                cards.len(),
                n(Rag::Red),
                n(Rag::Amber),
                n(Rag::Green)
            );
            Ok(())
        }
        Cmd::Portfolio { folder, out } => {
            if !folder.is_dir() {
                return Err(format!("{} is not a folder", folder.display()));
            }
            let pf = mpptool::portfolio::build(&folder, &opts, engine);
            let as_json = out.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"));
            let doc = if as_json {
                serde_json::to_string_pretty(&pf).map_err(|e| e.to_string())? + "\n"
            } else {
                mpptool::portfolio::to_html(&pf)
            };
            write_file(&out, &doc)?;
            eprintln!("wrote {} ({} plans, {} files skipped)", out.display(), pf.plans.len(), pf.skipped.len());
            Ok(())
        }
    }
}

fn money2(v: Option<f64>) -> String {
    v.map(|v| format!("{v:.2}")).unwrap_or_default()
}

fn print_variance_csv(v: &mpptool::variance::VarianceReport) -> Result<(), String> {
    let mut w = csv::Writer::from_writer(std::io::stdout());
    let h = [
        "id",
        "uid",
        "activity_id",
        "name",
        "outline_level",
        "summary",
        "baseline_start",
        "baseline_finish",
        "start",
        "finish",
        "start_slip_days",
        "finish_slip_days",
        "duration_variance_hours",
        "percent_complete",
        "bac",
        "pv",
        "ev",
        "ac",
    ];
    w.write_record(h).map_err(|e| e.to_string())?;
    for t in &v.tasks {
        let o = |x: Option<i64>| x.map(|x| x.to_string()).unwrap_or_default();
        w.write_record([
            opt_i(t.id),
            t.uid.to_string(),
            t.activity_id.clone().unwrap_or_default(),
            t.name.clone(),
            t.outline_level.to_string(),
            t.summary.to_string(),
            fmt_dt(&t.baseline_start),
            fmt_dt(&t.baseline_finish),
            fmt_dt(&t.start),
            fmt_dt(&t.finish),
            o(t.start_slip_days),
            o(t.finish_slip_days),
            t.duration_variance_hours.map(|x| x.to_string()).unwrap_or_default(),
            t.percent_complete.to_string(),
            money2(t.bac),
            money2(t.pv),
            money2(t.ev),
            money2(t.ac),
        ])
        .map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())
}

fn print_variance(v: &mpptool::variance::VarianceReport) {
    println!(
        "Baseline variance: {} tasks with a baseline, {} finishing late (max {} wd), project finish slip {} working days, status date {}",
        v.tasks_with_baseline,
        v.late_finishes,
        v.max_finish_slip_days.map(|d| format!("{d:+}")).unwrap_or_else(|| "–".into()),
        v.project_finish_slip_days.map(|d| format!("{d:+}")).unwrap_or_else(|| "–".into()),
        v.status_date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_else(|| "none".into())
    );
    if v.tasks.is_empty() {
        println!("(the plan has no baseline)");
        return;
    }
    println!(
        "{:>5} {:<34} {:<10} {:<10} {:>6} {:>6} {:>5} {:>11} {:>11} {:>11}",
        "ID", "Name", "BL finish", "Finish", "Start±", "Fin±", "%", "PV", "EV", "AC"
    );
    for t in &v.tasks {
        let o = |x: Option<i64>| x.map(|x| format!("{x:+}")).unwrap_or_default();
        let d = |x: Option<chrono::NaiveDateTime>| x.map(|x| x.format("%Y-%m-%d").to_string()).unwrap_or_default();
        let indent = "  ".repeat((t.outline_level - 1).clamp(0, 6) as usize);
        println!(
            "{:>5} {:<34} {:<10} {:<10} {:>6} {:>6} {:>5.0} {:>11} {:>11} {:>11}",
            opt_i(t.id),
            trunc(&format!("{indent}{}", t.name), 34),
            d(t.baseline_finish),
            d(t.finish),
            o(t.start_slip_days),
            o(t.finish_slip_days),
            t.percent_complete,
            money2(t.pv),
            money2(t.ev),
            money2(t.ac)
        );
    }
    match &v.earned_value {
        Some(e) => {
            let r = |x: Option<f64>| x.map(|x| format!("{x:.2}")).unwrap_or_else(|| "–".into());
            println!(
                "Earned value: BAC {:.2}  PV {:.2}  EV {:.2}  AC {:.2}  SV {:.2}  CV {:.2}  SPI {}  CPI {}  EAC {}",
                e.bac,
                e.pv,
                e.ev,
                e.ac,
                e.sv,
                e.cv,
                r(e.spi),
                r(e.cpi),
                r(e.eac)
            );
        }
        None => println!("Earned value: not available (no baseline cost in the file)"),
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
