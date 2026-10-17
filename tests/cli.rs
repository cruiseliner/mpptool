//! End-to-end tests of the `mpptool` binary. Fixtures are generated from our own CSV, so they are
//! freely redistributable. Set `MPXJ_DATA=/path/to/mpxj/junit/data` to also run against MPXJ's sample corpus.
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mpptool"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpptool-test-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

fn run_ok(args: &[&str]) -> String {
    let out = bin().args(args).output().unwrap();
    assert!(out.status.success(), "mpptool {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// Each test gets its own output file: tests run in parallel and must not share paths.
fn plan_xml(tag: &str) -> PathBuf {
    let out = tmp(&format!("plan-{tag}.xml"));
    run_ok(&["from-csv", fixture("plan.csv").to_str().unwrap(), out.to_str().unwrap()]);
    out
}

#[test]
fn from_csv_produces_readable_mspdi() {
    let xml = plan_xml("a");
    let text = std::fs::read_to_string(&xml).unwrap();
    assert!(text.starts_with("<?xml"));
    assert!(text.contains("<Project xmlns=\"http://schemas.microsoft.com/project\">"));
    assert!(text.contains("<ConstraintType>4</ConstraintType>"), "CSV start should become SNET");
    assert!(text.contains("<Notes>API &amp; database</Notes>"));

    let s: serde_json::Value = serde_json::from_str(&run_ok(&["summary", "--json", xml.to_str().unwrap()])).unwrap();
    assert_eq!(s["task_count"], 10);
    assert_eq!(s["resource_count"], 4);
    assert_eq!(s["assignment_count"], 8);
    assert_eq!(s["relation_count"], 7);
    assert_eq!(s["finish"], "2026-11-11T17:00:00");
    assert!(s["critical_tasks"].as_u64().unwrap() >= 5);
}

#[test]
fn tasks_show_visible_ids_and_uids() {
    let xml = plan_xml("b");
    let csv = run_ok(&["tasks", "--csv", xml.to_str().unwrap()]);
    let mut lines = csv.lines();
    let hdr: Vec<&str> = lines.next().unwrap().split(',').collect();
    let pi = hdr.iter().position(|h| *h == "predecessors").unwrap();
    let ui = hdr.iter().position(|h| *h == "predecessor_uids").unwrap();
    let backend = lines.find(|l| l.contains("Backend")).unwrap();
    let cols: Vec<&str> = backend.split(',').collect();
    assert_eq!(cols[pi], "3SS+1d");
    assert_eq!(cols[ui], "3SS+1d");
    let json: serde_json::Value = serde_json::from_str(&run_ok(&["tasks", "--json", xml.to_str().unwrap()])).unwrap();
    assert_eq!(json[7]["predecessors"][0]["predecessor_id"], 6);
}

#[test]
fn round_trip_is_stable() {
    let xml = plan_xml("c");
    let xml2 = tmp("plan-c2.xml");
    run_ok(&["to-xml", xml.to_str().unwrap(), xml2.to_str().unwrap()]);
    let a = run_ok(&["tasks", "--json", xml.to_str().unwrap()]);
    let b = run_ok(&["tasks", "--json", xml2.to_str().unwrap()]);
    assert_eq!(a, b);
    let c1 = run_ok(&["calendars", "--json", xml.to_str().unwrap()]);
    let c2 = run_ok(&["calendars", "--json", xml2.to_str().unwrap()]);
    assert_eq!(c1, c2);
}

#[test]
fn calendar_exceptions_survive_round_trip() {
    // Hand-written MSPDI with a holiday; to-xml must preserve it.
    let src = tmp("holiday.xml");
    std::fs::write(&src, include_str!("fixtures/holiday.xml")).unwrap();
    let out = tmp("holiday-out.xml");
    run_ok(&["to-xml", src.to_str().unwrap(), out.to_str().unwrap()]);
    let cal: serde_json::Value = serde_json::from_str(&run_ok(&["calendars", "--json", out.to_str().unwrap()])).unwrap();
    let ex = &cal[0]["exceptions"];
    assert_eq!(ex.as_array().unwrap().len(), 1, "{cal}");
    assert_eq!(ex[0]["from"], "2026-12-25");
    assert_eq!(ex[0]["working"], false);
    let t: serde_json::Value = serde_json::from_str(&run_ok(&["tasks", "--json", out.to_str().unwrap()])).unwrap();
    assert_eq!(t[0]["deadline"], "2027-01-08T17:00:00");
    assert_eq!(t[0]["notes"], "Check holidays");
    assert_eq!(t[0]["constraint_type"], 4);
}

#[test]
fn errors_are_clean() {
    let out = bin().args(["summary", "/nonexistent.mpp"]).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with("error: "), "{err}");
    let junk = tmp("junk.txt");
    std::fs::write(&junk, "hello").unwrap();
    let out = bin().args(["summary", junk.to_str().unwrap()]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("unrecognised format"));
    let out = bin().args(["--locale", "xx", "summary", junk.to_str().unwrap()]).output().unwrap();
    assert!(!out.status.success());
}

/// Optional: run against MPXJ's test corpus when MPXJ_DATA points at it (CI does this).
#[test]
fn mpxj_corpus_if_available() {
    let Ok(dir) = std::env::var("MPXJ_DATA") else { return };
    let mut files = vec![];
    for name in ["mpp14baseline.mpp", "mpp12calendar.mpp", "mpp9exceptions.mpp", "mpp14resource.mpp", "sample.mpx"] {
        let p = Path::new(&dir).join(name);
        if p.exists() {
            files.push(p);
        }
    }
    assert!(!files.is_empty(), "MPXJ_DATA set but no sample files found");
    for f in files {
        let out = tmp("corpus.xml");
        run_ok(&["summary", f.to_str().unwrap()]);
        run_ok(&["to-xml", f.to_str().unwrap(), out.to_str().unwrap()]);
        let a: serde_json::Value = serde_json::from_str(&run_ok(&["summary", "--json", f.to_str().unwrap()])).unwrap();
        let b: serde_json::Value = serde_json::from_str(&run_ok(&["summary", "--json", out.to_str().unwrap()])).unwrap();
        for k in ["task_count", "resource_count", "assignment_count", "relation_count", "calendar_exception_count"] {
            assert_eq!(a[k], b[k], "{k} mismatch for {}", f.display());
        }
    }
    let de = Path::new(&dir).join("sample.de.mpx");
    if de.exists() {
        run_ok(&["--locale", "de", "summary", de.to_str().unwrap()]);
    }
}

#[test]
fn engine_flag_compatibility() {
    let f = fixture("holiday.xml");
    run_ok(&["--engine", "mpxj", "summary", f.to_str().unwrap()]);
    let out = bin().args(["--engine", "mpxrust", "summary", f.to_str().unwrap()]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("removed in mpptool 0.6.1"));
}

#[test]
fn from_csv_writes_float_into_xml() {
    let xml = plan_xml("float");
    let text = std::fs::read_to_string(&xml).unwrap();
    assert!(text.contains("<TotalSlack>"), "TotalSlack should be written");
    assert!(text.contains("<FreeSlack>"), "FreeSlack should be written");
    // Backend (3SS+1, 6d) has 8h of total slack = 4800 tenths of a minute, read back by the MSPDI reader.
    let tasks: serde_json::Value = serde_json::from_str(&run_ok(&["tasks", "--json", xml.to_str().unwrap()])).unwrap();
    let backend = tasks.as_array().unwrap().iter().find(|t| t["name"] == "Backend").unwrap();
    assert!(backend["total_slack_hours"].as_f64().unwrap() > 0.0);
    // DCMA float checks no longer skip.
    let chk: serde_json::Value = serde_json::from_str(&run_ok(&["check", "--json", xml.to_str().unwrap()])).unwrap();
    let s = chk.to_string();
    assert!(!s.contains("no total float in the file"), "float checks should run: {s}");
}

#[test]
fn from_csv_honours_us_federal_holidays_and_custom_file() {
    let out = tmp("plan-us.xml");
    let o = out.to_str().unwrap();
    let csv = fixture("plan.csv");
    run_ok(&["from-csv", csv.to_str().unwrap(), o, "--holidays", "US", "--years", "2026-2027"]);
    let s: serde_json::Value = serde_json::from_str(&run_ok(&["summary", "--json", o])).unwrap();
    // Veterans Day (Wed 2026-11-11) pushes the finish one working day.
    assert_eq!(s["finish"], "2026-11-12T17:00:00");
    // 11 in 2026, 12 in 2027 (New Year's Day 2028 is observed on Friday 2027-12-31).
    assert_eq!(s["calendar_exception_count"], 23);
    let cals = run_ok(&["calendars", o]);
    assert!(cals.contains("2026-11-26") && cals.contains("Thanksgiving"), "{cals}");
    assert!(cals.contains("2027-07-05"), "Independence Day 2027 is observed Monday: {cals}");

    let out2 = tmp("plan-custom.xml");
    let o2 = out2.to_str().unwrap();
    let hf = fixture("shutdown.csv");
    run_ok(&["from-csv", csv.to_str().unwrap(), o2, "--holiday-file", hf.to_str().unwrap()]);
    let cals = run_ok(&["calendars", o2]);
    assert!(cals.contains("Year-end shutdown") && cals.contains("2026-11-28: working (Overtime Saturday)"), "{cals}");
    let text = std::fs::read_to_string(&out2).unwrap();
    assert!(text.contains("<FromTime>07:00:00</FromTime>") && text.contains("<ToTime>15:00:00</ToTime>"));
}

#[test]
fn from_csv_six_day_weeks_and_overtime_shifts() {
    let csv = fixture("plan.csv");
    let run = |tag: &str, extra: &[&str]| -> serde_json::Value {
        let out = tmp(&format!("plan-ww-{tag}.xml"));
        let mut args = vec!["from-csv", csv.to_str().unwrap(), out.to_str().unwrap()];
        args.extend_from_slice(extra);
        run_ok(&args);
        serde_json::from_str(&run_ok(&["summary", "--json", out.to_str().unwrap()])).unwrap()
    };
    let base = run("std", &[]);
    let six = run("six", &["--work-days", "mon-sat"]);
    let ten = run("ten", &["--hours", "06:00-11:00,11:30-16:30"]);
    let ot = run("ot", &["--day", "sat=07:00-12:00"]);
    let f = |v: &serde_json::Value| v["finish"].as_str().unwrap().to_string();
    assert_eq!(f(&base), "2026-11-11T17:00:00");
    assert!(f(&six) < f(&base) && f(&ten) < f(&base) && f(&ot) < f(&base), "{} {} {}", f(&six), f(&ten), f(&ot));
    assert!(f(&six) < f(&ot), "full Saturdays finish before half-day Saturday overtime");

    let out = tmp("plan-ww-xml.xml");
    run_ok(&["from-csv", csv.to_str().unwrap(), out.to_str().unwrap(), "--work-days", "all", "--hours", "07:00-19:00"]);
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("<DayType>1</DayType>\n\t\t\t\t\t<DayWorking>1</DayWorking>"), "Sunday should be working");
    assert!(text.contains("<FromTime>07:00:00</FromTime>") && text.contains("<ToTime>19:00:00</ToTime>"));
    assert!(text.contains("<Name>Project Calendar</Name>"));
}

#[test]
fn from_csv_rejects_bad_calendar_options() {
    let csv = fixture("plan.csv");
    let out = tmp("plan-bad.xml");
    for bad in [
        vec!["--holidays", "ZZ"],
        vec!["--work-days", "funday"],
        vec!["--hours", "17:00-08:00"],
        vec!["--day", "sat"],
        vec!["--years", "2028-2026"],
        vec!["--hours-per-day", "0"],
    ] {
        let mut args = vec!["from-csv", csv.to_str().unwrap(), out.to_str().unwrap()];
        args.extend_from_slice(&bad);
        let o = bin().args(&args).output().unwrap();
        assert!(!o.status.success(), "{bad:?} should fail");
        assert!(!String::from_utf8_lossy(&o.stderr).is_empty());
    }
}

#[test]
fn holidays_command_lists_us_federal() {
    let out = run_ok(&["holidays", "--country", "US", "--years", "2026", "--csv"]);
    assert!(out.starts_with("date,weekday,country,name"));
    assert_eq!(out.lines().count(), 12);
    assert!(out.contains("2026-07-03,Fri,US,\"Independence Day\""));
    let j: serde_json::Value = serde_json::from_str(&run_ok(&["holidays", "--country", "GB,DE", "--years", "2026", "--json"])).unwrap();
    assert_eq!(j.as_array().unwrap().len(), 8 + 9);
    assert!(run_ok(&["holidays", "--country", "list"]).contains("MX"));
}

#[test]
fn risk_command_reports_percentiles() {
    let csv = fixture("plan.csv");
    let j: serde_json::Value =
        serde_json::from_str(&run_ok(&["risk", csv.to_str().unwrap(), "--runs", "500", "--json", "--holidays", "US"])).unwrap();
    assert_eq!(j["runs"], 500);
    assert_eq!(j["deterministic_finish"], "2026-11-12T17:00:00");
    let p: Vec<&str> = j["percentiles"].as_array().unwrap().iter().map(|p| p["label"].as_str().unwrap()).collect();
    assert_eq!(p, ["P10", "P50", "P80", "P90"]);
    let tasks = j["tasks"].as_array().unwrap();
    assert!(tasks.iter().any(|t| t["name"] == "QA & fixes" && t["criticality"] == 1.0));
    let text = run_ok(&["risk", csv.to_str().unwrap(), "--runs", "200"]);
    assert!(text.contains("P80:") && text.contains("Criticality index"));
}

#[test]
fn risk_and_cpm_run_on_imported_schedules() {
    for f in ["status.xml", "sample-p6.xml", "multi-project.xer", "holiday.xml"] {
        let p = fixture(f);
        let j: serde_json::Value = serde_json::from_str(&run_ok(&["risk", p.to_str().unwrap(), "--runs", "200", "--json"])).unwrap();
        assert_eq!(j["percentiles"].as_array().unwrap().len(), 4, "{f}");
        let c: serde_json::Value = serde_json::from_str(&run_ok(&["cpm", p.to_str().unwrap(), "--json"])).unwrap();
        assert!(c.as_array().unwrap().iter().any(|r| r["critical"] == true), "{f}: no critical path");
    }
    // The project calendar's holidays are honoured: holiday.xml's 5-day task spans Christmas.
    let c: serde_json::Value = serde_json::from_str(&run_ok(&["cpm", fixture("holiday.xml").to_str().unwrap(), "--json"])).unwrap();
    assert_eq!(c[0]["early_finish"], "2026-12-29T17:00:00");
    // `tasks --json` output is accepted as input.
    let json = tmp("tasks-in.json");
    std::fs::write(&json, run_ok(&["tasks", "--json", fixture("status.xml").to_str().unwrap()])).unwrap();
    let a: serde_json::Value = serde_json::from_str(&run_ok(&["cpm", json.to_str().unwrap(), "--json"])).unwrap();
    let b: serde_json::Value = serde_json::from_str(&run_ok(&["cpm", fixture("status.xml").to_str().unwrap(), "--json"])).unwrap();
    assert_eq!(a.as_array().unwrap().len(), b.as_array().unwrap().len());
}

#[test]
fn progress_update_and_status_date() {
    let out = tmp("plan-progress.xml");
    let o = out.to_str().unwrap();
    let csv = fixture("plan.csv");
    let prog = fixture("progress.csv");
    run_ok(&["from-csv", csv.to_str().unwrap(), o, "--progress", prog.to_str().unwrap(), "--status-date", "2026-10-23"]);
    let tasks: serde_json::Value = serde_json::from_str(&run_ok(&["tasks", "--json", o])).unwrap();
    let t = |n: &str| tasks.as_array().unwrap().iter().find(|t| t["name"] == n).unwrap().clone();
    assert_eq!(t("Wireframes")["percent_complete"], 100.0);
    assert_eq!(t("Wireframes")["actual_finish"], "2026-10-21T17:00:00");
    assert_eq!(t("Visual design")["finish"], "2026-10-27T17:00:00");
    assert_eq!(t("Backend")["finish"], "2026-10-29T17:00:00");
    let chk: serde_json::Value = serde_json::from_str(&run_ok(&["check", "--json", o])).unwrap();
    let c9 = chk["checks"].as_array().unwrap().iter().find(|c| c["number"] == 9).unwrap().clone();
    assert_eq!(c9["status"], "pass", "no invalid dates after a status update: {c9}");
    // Progress without a status date is rejected.
    let bad = bin().args(["from-csv", csv.to_str().unwrap(), o, "--progress", prog.to_str().unwrap()]).output().unwrap();
    assert!(!bad.status.success());
}

#[test]
fn risk_events_and_sensitivity_in_output() {
    let csv = fixture("plan.csv");
    let risks = fixture("risks.csv");
    let j: serde_json::Value = serde_json::from_str(&run_ok(&[
        "risk",
        csv.to_str().unwrap(),
        "--runs",
        "1000",
        "--json",
        "--risks",
        risks.to_str().unwrap(),
        "--class",
        "labor=0.9,1.3",
    ]))
    .unwrap();
    let ev = j["events"].as_array().unwrap();
    assert_eq!(ev.len(), 2);
    assert!(ev.iter().all(|e| e["mean_finish_impact_days"].as_f64().unwrap() > 0.0));
    assert!(j["tasks"].as_array().unwrap().iter().any(|t| t["sensitivity"].as_f64().unwrap() > 0.3));
    let text = run_ok(&["risk", csv.to_str().unwrap(), "--runs", "300", "--risks", risks.to_str().unwrap()]);
    assert!(text.contains("Top drivers by sensitivity") && text.contains("Risk events"));
}

#[test]
fn check_thresholds_and_endpoint_exclusion() {
    let xml = plan_xml("thresholds");
    let x = xml.to_str().unwrap();
    let get = |args: &[&str]| -> serde_json::Value {
        let mut a = vec!["check", "--json", x];
        a.extend_from_slice(args);
        let v: serde_json::Value = serde_json::from_str(&run_ok(&a)).unwrap();
        v["checks"][0].clone()
    };
    let base = get(&[]);
    let ex = get(&["--exclude-endpoints"]);
    assert!(ex["count"].as_u64().unwrap() < base["count"].as_u64().unwrap());
    let th = get(&["--thresholds", "logic=50"]);
    assert_eq!(th["threshold"], "<= 50%");
    assert!(bin().args(["check", x, "--thresholds", "bogus=1"]).output().unwrap().status.code() != Some(0));
}

#[test]
fn from_csv_level_flag_reports() {
    let out = tmp("team-level.xml");
    let o = bin().args(["from-csv", fixture("team.csv").to_str().unwrap(), out.to_str().unwrap(), "--level"]).output().unwrap();
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("leveling:"));
}

#[test]
fn compare_counts_slips_on_the_plan_calendar() {
    let mk = |tag: &str, start: &str| {
        let c = tmp(&format!("cal-cmp-{tag}.csv"));
        std::fs::write(&c, format!("activity_id,name,start,duration_days,predecessors\nA10,Start,{start},1,\nA20,Work,,5,A10\n")).unwrap();
        let x = tmp(&format!("cal-cmp-{tag}.xml"));
        run_ok(&["from-csv", c.to_str().unwrap(), x.to_str().unwrap(), "--holidays", "US"]);
        x
    };
    let (a, b) = (mk("a", "2026-11-23"), mk("b", "2026-11-25"));
    let out = run_ok(&["compare", a.to_str().unwrap(), b.to_str().unwrap()]);
    // A20 start moves 11-24 -> 11-27 across Thanksgiving: 2 working days, not 3.
    assert!(out.contains("start: 2026-11-24 08:00 → 2026-11-27 08:00  (+2 wd)"), "{out}");
    assert!(out.contains("matched by activity_id"), "{out}");
}

#[test]
fn resources_load_uses_ten_hour_days_and_saturday_overtime() {
    let x = tmp("resload-10h.xml");
    let fx = fixture("team.csv");
    run_ok(&["from-csv", fx.to_str().unwrap(), x.to_str().unwrap(), "--hours", "06:00-11:00,11:30-16:30", "--day", "sat=07:00-12:00"]);
    assert!(std::fs::read_to_string(&x).unwrap().contains("<MinutesPerWeek>3300</MinutesPerWeek>"));
    let c = tmp("resload-10h.csv");
    run_ok(&["resources-load", x.to_str().unwrap(), c.to_str().unwrap()]);
    let t = std::fs::read_to_string(&c).unwrap();
    assert!(t.contains("2,Bob,2026-05-04,54.4,55,false"), "Bob fits a 55 h week: {t}");
    assert!(t.contains("1,Alice,2026-05-04,80,55,true"), "{t}");
}

#[test]
fn activity_ids_drive_predecessors_progress_and_risks() {
    let c = tmp("aid.csv");
    std::fs::write(
        &c,
        "activity_id,name,start,duration_days,predecessors\nA10,Dig,2026-06-01,5,\nA20,Pour,,5,A10\nA30,Cure,,3,A20FS+1;#1\n",
    )
    .unwrap();
    let x = tmp("aid.xml");
    run_ok(&["from-csv", c.to_str().unwrap(), x.to_str().unwrap()]);
    let tasks = run_ok(&["tasks", x.to_str().unwrap()]);
    assert!(tasks.contains("2FS+1d;1"), "{tasks}");
    assert!(std::fs::read_to_string(&x).unwrap().contains("<Value>A30</Value>"), "activity ID written to Text1");
    let p = tmp("aid-progress.csv");
    std::fs::write(&p, "id,actual_start,actual_finish,percent_complete\nA10,2026-06-01,2026-06-05,100\n2,2026-06-08,,50\n").unwrap();
    let r = tmp("aid-risks.csv");
    std::fs::write(
        &r,
        "id,probability,min_days,likely_days,max_days,tasks,cost_min,cost_likely,cost_max\nR1,50%,1,2,3,A20,1000,2000,5000\n",
    )
    .unwrap();
    let out = run_ok(&[
        "risk",
        c.to_str().unwrap(),
        "--progress",
        p.to_str().unwrap(),
        "--status-date",
        "2026-06-10",
        "--risks",
        r.to_str().unwrap(),
        "--runs",
        "500",
    ]);
    assert!(out.contains("Risk-event cost"), "{out}");
    let bad = tmp("aid-bad.csv");
    std::fs::write(&bad, "activity_id,name,duration_days,predecessors\nA1,x,1,\nA1,y,1,\n").unwrap();
    assert!(!bin().args(["from-csv", bad.to_str().unwrap(), tmp("aid-bad.xml").to_str().unwrap()]).output().unwrap().status.success());
}
